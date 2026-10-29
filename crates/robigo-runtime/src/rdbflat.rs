//! Flat (pre-inflated) cache for lazy-load databases (`.rdb`).
//!
//! A package's `.rdb` stores every object as its own compressed stream;
//! `lazyLoadDBfetch` inflates one per fetch, and loading the default
//! packages costs ~77M instructions in the inflater alone (zlib-rs:
//! Huffman table builds per stream, adler32 over the output, ...). GNU R
//! pays the same per fetch.
//!
//! `scripts/build-rdbflat.sh` records a battery of robigo runs
//! (`scripts/build-rdbflat.py`) and writes, per `.rdb`, a flat file with
//! every fetched entry pre-inflated:
//!
//! ```text
//! "RFLAT01\n"                      8 bytes
//! u64 rdb_size, u64 rdb_mtime_ns   guard: the cache is only used while
//!                                   the rdb is exactly the file it was
//!                                   built from
//! u64 path_len, path               the rdb this cache serves
//! u64 n
//! n x {u64 off, u32 clen, u32 ulen, u64 foff}   index (clen is the
//!                                               compressed length, i.e.
//!                                               the second half of the
//!                                               fetch key)
//! the uncompressed entries, entry i at foff..foff+ulen
//! ```
//!
//! `with_entry` serves an entry as a zero-copy slice; a missing or stale
//! cache falls back to the per-fetch path, so behaviour is byte-identical
//! either way (entries the battery did not record take that path too).
//! The same format caches a lazy-load database's `.rdx` index — a gzip
//! file `lazyload.R`'s `readRDS` inflates whole at every open — as a
//! single `(0, 0)` entry claimed on the `.rdx` itself; `builtins/io.rs`
//! serves it at the gzfile open, falling back to the decoder.
//! Enabled by pointing `ROBIGO_RDBFLAT` at the cache directory, or, when
//! that is unset, by a `rdbflat/` directory beside the executable's
//! parent's parent (`.robigo/bin/robigo`, `.robigo/rdbflat` — the layout
//! `scripts/build-rdbflat.sh` writes, as `scripts/build-natives.sh` does
//! for natives); absent, the module costs a `getenv` and a `read_dir`.
//! Flat files are read lazily, one at the first fetch from their rdb —
//! a NULL-packages run reads base.rdb's cache and nothing else.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// A multiply-xor hasher (FxHash) for the flat index: it is rebuilt once
/// per process and probed a few thousand times, and SipHash's
/// finalization costs more than the lookup itself.
#[derive(Default)]
struct Fx(u64);

const FX_SEED: u64 = 0x517cc1b727220a95;

impl Hasher for Fx {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(FX_SEED);
        }
    }
    fn write_u32(&mut self, i: u32) {
        self.0 = (self.0.rotate_left(5) ^ i as u64).wrapping_mul(FX_SEED);
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(FX_SEED);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

/// (off, clen) -> (foff, ulen), FxHashed.
type FlatIndex = HashMap<(u64, u32), (u64, u32), BuildHasherDefault<Fx>>;

struct Flat {
    data: Vec<u8>,
    index: FlatIndex,
}

struct Cache {
    /// rdb path -> the flat file claiming it, with its (size, mtime_ns)
    /// guard
    claims: HashMap<PathBuf, (PathBuf, u64, u64)>,
    /// rdb path -> the loaded flat
    loaded: HashMap<PathBuf, Rc<Flat>>,
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

impl Cache {
    /// Read the flat directory: each file's header only. `$ROBIGO_RDBFLAT`
    /// names it; unset, `rdbflat/` beside the executable's parent's parent
    /// (`.robigo/bin/robigo` → `.robigo/rdbflat`) is used when present.
    fn open() -> Cache {
        let dir = match std::env::var_os("ROBIGO_RDBFLAT") {
            Some(d) => PathBuf::from(d),
            None => {
                let Some(dir) = std::env::current_exe().ok().and_then(|e| {
                    let root = e.parent().and_then(|p| p.parent())?;
                    Some(root.join("rdbflat"))
                }) else {
                    return Cache { claims: HashMap::new(), loaded: HashMap::new() };
                };
                dir
            }
        };
        let mut claims = HashMap::new();
        let Ok(entries) = std::fs::read_dir(dir) else { return Cache { claims, loaded: HashMap::new() } };
        for e in entries.flatten() {
            // the header: magic, guard, path (the path starts at byte 32)
            let Ok(mut f) = std::fs::File::open(e.path()) else { continue };
            use std::io::Read;
            let mut head = [0u8; 32];
            if f.read_exact(&mut head).is_err() || &head[..8] != b"RFLAT01\n" {
                continue;
            }
            let plen = u64_at(&head, 24) as usize;
            if plen == 0 || plen > 4096 {
                continue;
            }
            let mut path = vec![0u8; plen];
            if f.read_exact(&mut path).is_err() {
                continue;
            }
            let Ok(path) = String::from_utf8(path) else { continue };
            claims.insert(PathBuf::from(path), (e.path(), u64_at(&head, 8), u64_at(&head, 16)));
        }
        Cache { claims, loaded: HashMap::new() }
    }

    /// The flat for `file`, loading it on first use; `None` when there is
    /// no claim, the rdb no longer matches the guard, or the file is
    /// unusable.
    fn flat(&mut self, file: &Path) -> Option<Rc<Flat>> {
        if let Some(flat) = self.loaded.get(file) {
            return Some(flat.clone());
        }
        let (path, size, mtime_ns) = self.claims.get(file)?.clone();
        let flat = load(&path, file, size, mtime_ns)?;
        let flat = Rc::new(flat);
        self.loaded.insert(file.to_path_buf(), flat.clone());
        Some(flat)
    }
}

/// Read a flat file, checking the rdb guard: it must still be the exact
/// file (size and mtime) the cache was built from.
fn load(path: &Path, rdb: &Path, size: u64, mtime_ns: u64) -> Option<Flat> {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::metadata(rdb) else { return None };
    let mtime = (meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128) as u64;
    if meta.len() != size || mtime != mtime_ns {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let plen = u64_at(&bytes, 24) as usize;
    let index = parse_index(&bytes, 32 + plen)?;
    Some(Flat { data: bytes, index })
}

/// The entry index at `p` (just past the path); every entry's slice must
/// lie inside the file.
fn parse_index(bytes: &[u8], mut p: usize) -> Option<FlatIndex> {
    if p + 8 > bytes.len() {
        return None;
    }
    let n = u64_at(bytes, p) as usize;
    p += 8;
    if p.checked_add(n.checked_mul(24)?)? > bytes.len() {
        return None;
    }
    let mut index = FlatIndex::with_capacity_and_hasher(n, Default::default());
    for _ in 0..n {
        let off = u64_at(bytes, p);
        let clen = u32_at(bytes, p + 8);
        let ulen = u32_at(bytes, p + 12);
        let foff = u64_at(bytes, p + 16);
        if foff.checked_add(ulen as u64)? > bytes.len() as u64 {
            return None;
        }
        index.insert((off, clen), (foff, ulen));
        p += 24;
    }
    Some(index)
}

/// Record a fetch for `scripts/build-rdbflat.py` when ROBIGO_RDBLOG names
/// a file (recording runs only; the env is read once per process).
pub fn log_fetch(file: &str, off: usize, len: usize, compressed: i32) {
    use std::cell::RefCell;
    use std::io::Write;
    // None: not resolved yet; Some(None): no log; Some(Some(f)): the log
    thread_local! {
        static LOG: RefCell<Option<Option<std::fs::File>>> = const { RefCell::new(None) };
    }
    LOG.with(|log| {
        let mut slot = log.borrow_mut();
        if slot.is_none() {
            *slot = Some(
                std::env::var_os("ROBIGO_RDBLOG")
                    .and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok()),
            );
        }
        if let Some(Some(log)) = slot.as_mut() {
            let _ = writeln!(log, "{}\t{}\t{}\t{}", file, off, len, compressed);
        }
    });
}

/// Serve the entry `(off, len)` of `file`'s flat cache to `f`, or `None`
/// when there is no usable cache for it.
///
/// The entry's slice is borrowed from a thread-local `Rc` handle, not a
/// held lock: unserializing an entry can call the environment hook, which
/// fetches reference environments from the same database — the nested
/// fetch must be able to re-enter.
pub fn with_entry<T>(file: &str, off: usize, len: usize, f: impl FnOnce(&[u8]) -> T) -> Option<T> {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Option<Cache>> = const { RefCell::new(None) };
    }
    let flat = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.get_or_insert_with(Cache::open).flat(Path::new(file))
    })?;
    let (foff, ulen) = flat.index.get(&(off as u64, len as u32))?;
    let end = *foff as usize + *ulen as usize;
    Some(f(&flat.data[*foff as usize..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat image in the format scripts/build-rdbflat.py writes.
    fn flat(path: &str, entries: &[((u64, u32), &[u8])]) -> Vec<u8> {
        let path = path.as_bytes();
        let mut b = Vec::new();
        b.extend_from_slice(b"RFLAT01\n");
        b.extend_from_slice(&12345u64.to_le_bytes()); // rdb size
        b.extend_from_slice(&678910u64.to_le_bytes()); // rdb mtime_ns
        b.extend_from_slice(&(path.len() as u64).to_le_bytes());
        b.extend_from_slice(path);
        b.extend_from_slice(&(entries.len() as u64).to_le_bytes());
        let data_start = b.len() + 24 * entries.len();
        let mut data = Vec::new();
        for ((off, clen), raw) in entries {
            b.extend_from_slice(&off.to_le_bytes());
            b.extend_from_slice(&clen.to_le_bytes());
            b.extend_from_slice(&(raw.len() as u32).to_le_bytes());
            b.extend_from_slice(&((data_start + data.len()) as u64).to_le_bytes());
            data.extend_from_slice(raw);
        }
        b.extend_from_slice(&data);
        b
    }

    #[test]
    fn parses_the_index_and_bounds_checks() {
        let entries: Vec<((u64, u32), &[u8])> = vec![(0u64, 3u32, b"abc".as_slice()), (17, 2, b"de".as_slice())]
            .into_iter()
            .map(|(o, c, r)| ((o, c), r))
            .collect();
        let bytes = flat("/usr/lib/R/library/base/R/base.rdb", &entries);
        let plen = u64_at(&bytes, 24) as usize;
        let index = parse_index(&bytes, 32 + plen).unwrap();
        let (foff, ulen) = index[&(0, 3)];
        assert_eq!(&bytes[foff as usize..foff as usize + ulen as usize], b"abc");
        let (foff, ulen) = index[&(17, 2)];
        assert_eq!(&bytes[foff as usize..foff as usize + ulen as usize], b"de");
        assert_eq!(index.len(), 2);

        // truncated files are rejected
        assert!(parse_index(&bytes[..bytes.len() - 1], 32 + plen).is_none());

        // an entry whose data points past the end is rejected
        let entries: Vec<((u64, u32), &[u8])> = vec![((5, 4), b"z")];
        let mut past = flat("x", &entries);
        let e = 32 + 1 + 8 + 16; // first entry's foff field
        let far = (past.len() as u64 + 10).to_le_bytes();
        past[e..e + 8].copy_from_slice(&far);
        assert!(parse_index(&past, 32 + 1).is_none());
    }
}
