//! connections.c, the part serialization needs (L1): the connection table
//! (`Connections[]`, 128 slots, 0 to 2 the standard ones), `file()` and
//! `gzfile()` (`do_url`'s file branch, `do_gzfile`), `gzcon()` and
//! `close()`, and the reads and writes `serialize.c`'s connection streams
//! make (`R_ReadConnection`, `R_WriteConnection`, a connection's `read` and
//! `write`).
//!
//! The table lives in the isolate ([`Cx::ext`], docs/g3.md §11). A
//! connection object is GNU R's: the slot number as an integer of class
//! `c(<class>, "connection")` with a `conn_id` attribute, an external
//! pointer to the connection's id.
//!
//! What is not here says so when reached ("robigo: ... is not implemented
//! yet"): URLs, clipboards and fifos, `bzfile`/`xzfile`/`zstdfile` (and
//! `file()`/`gzfile()` of a file compressed that way), re-encoding
//! (`encoding` other than `""` and `"native.enc"` on a text connection),
//! and the finalizer that closes a connection nothing refers to (GNU R's
//! "closing unused connection" warning).
//!
//! gzip: GNU R's own `gzio.h` (a gzip header with no name and no time, OS
//! code 3, raw deflate, the CRC-32 and length trailer; reading accepts a
//! file without the magic as uncompressed, and concatenated members) over
//! the zlib-rs backend. Deflate's output is a valid stream of the same
//! data but not byte for byte zlib's at the same level: zlib-rs makes
//! different choices than GNU R's zlib (its window and memory level are
//! the same), so a file `gzfile` writes may differ from GNU R's in its
//! compressed bytes, never in what it decompresses to.

use flate2::{Compress, Compression, Crc, Decompress, FlushCompress, FlushDecompress, Status};
use robigo_heap::types::*;
use robigo_rtbuiltins::attrib::{Attr, Attrs};
use robigo_rtcore::{Args, Char, Cx, Encoding, Global, Out, RResult, RSym, Ref, Root};
use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

/// `NCONNECTIONS`.
const NCONNECTIONS: usize = 128;
/// gzio.h's `Z_BUFSIZE`.
const Z_BUFSIZE: usize = 16384;
/// gzio.h's `OS_CODE` (Unix).
const OS_CODE: u8 = 0x03;
const NA_INTEGER: i32 = i32::MIN;

/// The isolate's connection table.
pub struct Connections {
    slots: RefCell<Vec<Option<Box<Conn>>>>,
    /// `current_id`: the last connection id handed out.
    current_id: Cell<usize>,
}

impl Connections {
    pub fn new() -> Connections {
        let mut slots: Vec<Option<Box<Conn>>> = (0..NCONNECTIONS).map(|_| None).collect();
        for (i, d) in ["stdin", "stdout", "stderr"].iter().enumerate() {
            let mut c = Conn::new("terminal", d.as_bytes(), if i == 0 { "r" } else { "w" }, Imp::Std);
            c.isopen = true;
            c.canread = i == 0;
            c.canwrite = i != 0;
            slots[i] = Some(Box::new(c));
        }
        Connections { slots: RefCell::new(slots), current_id: Cell::new(3) }
    }
}

impl Default for Connections {
    fn default() -> Connections {
        Connections::new()
    }
}

fn table(cx: &Cx) -> &Connections {
    cx.ext::<Connections>().expect("robigo: the connection table is not installed (robigo_rt::install)")
}

/// One connection (GNU R's `struct Rconn`, the fields robigo uses).
pub struct Conn {
    pub class: &'static str,
    pub description: Vec<u8>,
    /// at most 4 characters, as GNU R keeps it
    pub mode: String,
    pub encname: String,
    pub isopen: bool,
    pub canread: bool,
    pub canwrite: bool,
    pub canseek: bool,
    pub text: bool,
    pub blocking: bool,
    pub is_gzcon: bool,
    /// `status`: what closing returned (NA until a file is closed).
    pub status: i32,
    pub id: usize,
    /// `ex_ptr`: the `conn_id` external pointer.
    ex_ptr: Option<Global>,
    imp: Imp,
}

enum Imp {
    Std,
    File(FileConn),
    Gz(GzFile),
    Gzcon(Box<GzconConn>),
}

struct FileConn {
    fp: Option<File>,
    rpos: u64,
    wpos: u64,
    last_was_write: bool,
}

/// A gzfile's state once open.
struct GzFile {
    compress: u32,
    io: Option<GzIo>,
}

enum GzIo {
    Read(GzReader<File>),
    Write(GzWriter<File>),
}

/// `gzcon`'s private part: the wrapped connection and the stream.
struct GzconConn {
    inner: Box<Conn>,
    level: u32,
    allow: bool,
    io: Option<GzconIo>,
}

enum GzconIo {
    Read(GzReader<ConnReader>),
    Write(GzWriter<ConnWriter>),
}

/// The wrapped connection, read as a byte source.
struct ConnReader(*mut Conn);
struct ConnWriter(*mut Conn);

impl Read for ConnReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // SAFETY: the wrapped connection is boxed inside the gzcon that owns
        // this reader, which is only used while that gzcon is borrowed.
        #[allow(unsafe_code)]
        let c = unsafe { &mut *self.0 };
        conn_read(c, buf)
    }
}

impl Write for ConnWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // SAFETY: as for `ConnReader`.
        #[allow(unsafe_code)]
        let c = unsafe { &mut *self.0 };
        conn_write(c, buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Conn {
    // R: connections.c:init_con
    fn new(class: &'static str, description: &[u8], mode: &str, imp: Imp) -> Conn {
        Conn {
            class,
            description: description.to_vec(),
            mode: mode.chars().take(4).collect(),
            encname: String::new(),
            isopen: false,
            canread: true,
            canwrite: true,
            canseek: false,
            text: true,
            blocking: false,
            is_gzcon: false,
            status: NA_INTEGER,
            id: 0,
            ex_ptr: None,
            imp,
        }
    }
}

/// The open's failure: the warning GNU R gives before "cannot open the
/// connection".
type OpenResult = Result<(), String>;

/// `R_PATH_MAX`: `PATH_MAX` on Linux.
pub(crate) const R_PATH_MAX: usize = 4096;

/// `R_ExpandFileName_unix(s)`: a leading `~` or `~user` (up to the first
/// `/`) is that home directory: `HOME` for `~` (the password entry when it
/// is unset or empty), the password entry for `~user`. A name with no such
/// directory is left as it is, and so is one whose expansion would not fit
/// in `R_PATH_MAX`, with the text of GNU R's warning.
// R: sys-unix.c:R_ExpandFileName_unix (no readline: robigo-rt's and
// Rscript's case)
pub fn expand_file_name_unix(s: &[u8]) -> (Vec<u8>, Option<String>) {
    if s.first() != Some(&b'~') {
        return (s.to_vec(), None);
    }
    let slash = s[1..].iter().position(|&c| c == b'/').map(|k| k + 1);
    let user = &s[1..slash.unwrap_or(s.len())];
    let home: Option<Vec<u8>> = if user.is_empty() {
        // follow readline (and not libedit) in preferring HOME
        match std::env::var_os("HOME") {
            Some(h) if !h.is_empty() => Some(std::os::unix::ffi::OsStrExt::as_bytes(h.as_os_str()).to_vec()),
            _ => robigo_rtcore::clib::passwd_home(None),
        }
    } else {
        robigo_rtcore::clib::passwd_home(Some(user))
    };
    let Some(home) = home else { return (s.to_vec(), None) };
    match slash {
        // ~name
        None => (home, None),
        // ~name/path: "%s/%s" of the home and what follows the slash
        Some(k) => {
            let mut v = home;
            v.push(b'/');
            v.extend_from_slice(&s[k + 1..]);
            if v.len() >= R_PATH_MAX {
                let msg =
                    format!("expanded path length {} would be too long for\n{}\n", v.len(), String::from_utf8_lossy(s));
                return (s.to_vec(), Some(msg));
            }
            (v, None)
        }
    }
}

/// `R_ExpandFileName(name)`, its warning given.
pub fn r_expand_file_name(cx: &mut robigo_rtcore::Cx, name: &[u8]) -> robigo_rtcore::RResult<Vec<u8>> {
    let (v, w) = expand_file_name_unix(name);
    if let Some(w) = w {
        cx.warning(&w)?;
    }
    Ok(v)
}

/// `R_ExpandFileName(name)` without its over-long expansion warning: for
/// a connection's open callback, which has no `Cx`, and today also for
/// `file()`'s and `gzfile()`'s sniffing of the file's type (connections,
/// L's: docs/ledger/g3/2026-11-17-b5a-platform.md). Elsewhere,
/// `r_expand_file_name`.
pub fn expand_file_name(name: &[u8]) -> Vec<u8> {
    expand_file_name_unix(name).0
}

fn path_of(name: &[u8]) -> std::path::PathBuf {
    std::path::PathBuf::from(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(name))
}

/// `strerror(errno)` of an I/O error.
pub fn strerror(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(code) => {
            let s = std::io::Error::from_raw_os_error(code).to_string();
            match s.rfind(" (os error ") {
                Some(i) => s[..i].to_string(),
                None => s,
            }
        }
        None => e.to_string(),
    }
}

fn is_dir(name: &[u8]) -> bool {
    std::fs::metadata(path_of(name)).is_ok_and(|m| m.is_dir())
}

/// `fopen(name, mode)`'s file.
fn fopen(name: &[u8], mode: &str) -> std::io::Result<File> {
    let m = mode.as_bytes();
    let plus = m.get(1) == Some(&b'+') || m.get(2) == Some(&b'+');
    let mut o = std::fs::OpenOptions::new();
    match m.first() {
        Some(b'w') => {
            o.write(true).create(true).truncate(true).read(plus);
        }
        Some(b'a') => {
            o.append(true).create(true).read(plus);
        }
        _ => {
            o.read(true).write(plus);
        }
    }
    o.open(path_of(name))
}

// R: connections.c:file_open
fn file_open(c: &mut Conn) -> OpenResult {
    if c.description.is_empty() {
        return Err("robigo: 'file(\"\")' (an anonymous file) is not implemented yet".into());
    }
    let name = expand_file_name(&c.description);
    if name == b"stdin" {
        return Err("robigo: 'file(\"stdin\")' is not implemented yet".into());
    }
    let shown = String::from_utf8_lossy(&name).into_owned();
    let fp = match fopen(&name, &c.mode) {
        Ok(f) => f,
        Err(e) => return Err(format!("cannot open file '{shown}': {}", strerror(&e))),
    };
    if fp.metadata().is_ok_and(|m| m.is_dir()) {
        return Err(format!("cannot open file '{shown}': it is a directory"));
    }
    let m = c.mode.as_bytes();
    let mlen = m.len();
    c.isopen = true;
    c.canwrite = m[0] == b'w' || m[0] == b'a';
    c.canread = !c.canwrite;
    if mlen >= 2 && m[1] == b'+' {
        c.canread = true;
        c.canwrite = true;
    }
    c.text = !(mlen >= 2 && m[mlen - 1] == b'b');
    let mut fp = fp;
    let wpos = if c.canwrite { fp.stream_position().unwrap_or(0) } else { 0 };
    let canread = c.canread;
    if let Imp::File(f) = &mut c.imp {
        f.fp = Some(fp);
        f.rpos = 0;
        f.wpos = wpos;
        f.last_was_write = !canread;
    }
    Ok(())
}

/// What `comp_type_from_file` finds: 0 not compressed (or unreadable),
/// 1 gzip, 2 bzip2, 3 xz, 4 zstd.
// R: connections.c:comp_type_from_file
fn comp_type_from_file(name: &[u8]) -> u8 {
    let mut buf = [0u8; 7];
    let Ok(mut f) = File::open(path_of(name)) else { return 0 };
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    if n >= 2 && buf[0] == 0x1f && buf[1] == 0x8b {
        1
    } else if n >= 3 && &buf[..3] == b"BZh" {
        2
    } else if n >= 6 && buf[0] == 0xFD && &buf[1..6] == b"7zXZ\0" {
        3
    } else if n >= 5 && buf[0] == 0xFF && &buf[1..5] == b"LZMA" {
        3
    } else if n >= 5 && buf[0] == 0x5D && buf[1] == 0 && buf[2] == 0 && buf[3] == 0x80 && buf[4] == 0 {
        3
    } else if n >= 4 && buf[..4] == [0x28, 0xb5, 0x2f, 0xfd] {
        4
    } else {
        0
    }
}

// R: connections.c:gzfile_open
fn gzfile_open(c: &mut Conn) -> OpenResult {
    let name = expand_file_name(&c.description);
    let shown = String::from_utf8_lossy(&name).into_owned();
    if is_dir(&name) {
        return Err(format!("cannot open file '{shown}': it is a directory"));
    }
    let write = c.mode.contains('w') || c.mode.starts_with('a');
    let Imp::Gz(g) = &mut c.imp else { unreachable!() };
    let io = if write {
        let f = if c.mode.starts_with('a') { fopen(&name, "ab") } else { fopen(&name, "wb") };
        match f {
            Ok(f) => GzIo::Write(GzWriter::new(f, g.compress)),
            Err(e) => return Err(format!("cannot open compressed file '{shown}', probable reason '{}'", strerror(&e))),
        }
    } else {
        match File::open(path_of(&name)) {
            Ok(f) => GzIo::Read(GzReader::new(f, true)),
            Err(e) => return Err(format!("cannot open compressed file '{shown}', probable reason '{}'", strerror(&e))),
        }
    };
    g.io = Some(io);
    c.isopen = true;
    c.canwrite = c.mode.starts_with('w') || c.mode.starts_with('a');
    c.canread = !c.canwrite;
    c.text = !c.mode.contains('b');
    Ok(())
}

// R: connections.c:gzcon_open
fn gzcon_open(c: &mut Conn) -> Result<(), (String, bool)> {
    let Imp::Gzcon(g) = &mut c.imp else { unreachable!() };
    if !g.inner.isopen {
        open_conn(&mut g.inner).map_err(|m| (m, false))?;
    }
    c.isopen = true;
    c.canwrite = g.inner.canwrite;
    c.canread = !c.canwrite;
    let inner: *mut Conn = &mut *g.inner;
    if c.canread {
        let mut r = GzReader::new(ConnReader(inner), g.allow);
        match r.check_header() {
            Header::Transparent if !g.allow => {
                return Err(("file stream does not have gzip magic number".into(), true));
            }
            Header::Bad => return Err(("file stream does not have valid gzip header".into(), true)),
            _ => {}
        }
        g.io = Some(GzconIo::Read(r));
    } else {
        let mut w = ConnWriter(inner);
        let head = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, OS_CODE];
        let _ = w.write(&head);
        g.io = Some(GzconIo::Write(GzWriter::without_header(w, g.level)));
    }
    Ok(())
}

/// `con->open(con)`: false with the warning GNU R gives.
fn open_conn(c: &mut Conn) -> OpenResult {
    match c.imp {
        Imp::Std => Ok(()),
        Imp::File(_) => file_open(c),
        Imp::Gz(_) => gzfile_open(c),
        Imp::Gzcon(_) => match gzcon_open(c) {
            Ok(()) => Ok(()),
            Err((m, _)) => Err(m),
        },
    }
}

/// `con->close(con)`.
fn close_conn(c: &mut Conn) -> Option<String> {
    let mut err = None;
    match &mut c.imp {
        Imp::Std => {}
        Imp::File(f) => {
            if c.isopen {
                c.status = match f.fp.take() {
                    Some(fp) => match fp.sync_all().or(Ok::<(), std::io::Error>(())) {
                        Ok(()) => 0,
                        Err(_) => -1,
                    },
                    None => 0,
                };
            }
        }
        Imp::Gz(g) => {
            if let Some(GzIo::Write(w)) = g.io.take()
                && let Err(e) = w.finish()
            {
                err = Some(strerror(&e));
            }
        }
        Imp::Gzcon(g) => {
            if let Some(GzconIo::Write(w)) = g.io.take()
                && w.finish().is_err()
            {
                err = Some("writing error whilst flushing 'gzcon' connection".into());
            }
            g.io = None;
            if g.inner.isopen {
                close_conn(&mut g.inner);
                g.inner.isopen = false;
            }
        }
    }
    c.isopen = false;
    err
}

/// `con->read(buf, 1, n, con)`.
fn conn_read(c: &mut Conn, buf: &mut [u8]) -> std::io::Result<usize> {
    match &mut c.imp {
        Imp::Std => Err(std::io::Error::other("robigo: reading the standard connections is not implemented yet")),
        Imp::File(f) => {
            let fp = f.fp.as_mut().ok_or_else(|| std::io::Error::other("not open"))?;
            if f.last_was_write {
                f.wpos = fp.stream_position()?;
                f.last_was_write = false;
                fp.seek(SeekFrom::Start(f.rpos))?;
            }
            let mut n = 0;
            while n < buf.len() {
                match fp.read(&mut buf[n..]) {
                    Ok(0) => break,
                    Ok(k) => n += k,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(n)
        }
        Imp::Gz(g) => match &mut g.io {
            Some(GzIo::Read(r)) => r.read_full(buf),
            _ => Ok(0),
        },
        Imp::Gzcon(g) => match &mut g.io {
            Some(GzconIo::Read(r)) => r.read_full(buf),
            _ => Ok(0),
        },
    }
}

/// `con->write(buf, 1, n, con)`.
fn conn_write(c: &mut Conn, buf: &[u8]) -> std::io::Result<usize> {
    match &mut c.imp {
        Imp::Std => Err(std::io::Error::other("robigo: writing the standard connections is not implemented yet")),
        Imp::File(f) => {
            let fp = f.fp.as_mut().ok_or_else(|| std::io::Error::other("not open"))?;
            if !f.last_was_write {
                f.rpos = fp.stream_position()?;
                f.last_was_write = true;
                fp.seek(SeekFrom::Start(f.wpos))?;
            }
            fp.write_all(buf)?;
            f.wpos += buf.len() as u64;
            Ok(buf.len())
        }
        Imp::Gz(g) => match &mut g.io {
            Some(GzIo::Write(w)) => w.write_data(buf),
            _ => Ok(0),
        },
        Imp::Gzcon(g) => match &mut g.io {
            Some(GzconIo::Write(w)) => w.write_data(buf),
            _ => Ok(0),
        },
    }
}

// ---- gzip, as gzio.h has it -------------------------------------------------

enum Header {
    Ok,
    Transparent,
    Bad,
    Eof,
}

/// R_gzread: a gzip reader over `src` that takes a stream without the
/// magic as uncompressed (`transparent`, when `allow`), concatenated members,
/// and stops at trailing data that is not a member.
struct GzReader<R> {
    src: R,
    buf: Vec<u8>,
    pos: usize,
    end: usize,
    src_eof: bool,
    inflate: Decompress,
    crc: Crc,
    transparent: bool,
    done: bool,
    allow: bool,
    started: bool,
}

impl<R: Read> GzReader<R> {
    fn new(src: R, allow: bool) -> GzReader<R> {
        GzReader {
            src,
            buf: vec![0; Z_BUFSIZE],
            pos: 0,
            end: 0,
            src_eof: false,
            inflate: Decompress::new(false),
            crc: Crc::new(),
            transparent: false,
            done: false,
            allow,
            started: false,
        }
    }

    fn fill(&mut self) -> std::io::Result<()> {
        if self.pos < self.end || self.src_eof {
            return Ok(());
        }
        let n = self.src.read(&mut self.buf)?;
        self.pos = 0;
        self.end = n;
        if n == 0 {
            self.src_eof = true;
        }
        Ok(())
    }

    fn byte(&mut self) -> std::io::Result<Option<u8>> {
        self.fill()?;
        if self.pos == self.end {
            return Ok(None);
        }
        self.pos += 1;
        Ok(Some(self.buf[self.pos - 1]))
    }

    /// Two bytes in the buffer, if the source has them.
    fn peek2(&mut self) -> std::io::Result<Option<[u8; 2]>> {
        if self.end - self.pos < 2 {
            let rest = self.buf[self.pos..self.end].to_vec();
            self.buf[..rest.len()].copy_from_slice(&rest);
            self.pos = 0;
            self.end = rest.len();
            while self.end < 2 && !self.src_eof {
                let n = self.src.read(&mut self.buf[self.end..])?;
                if n == 0 {
                    self.src_eof = true;
                }
                self.end += n;
            }
            if self.end < 2 {
                return Ok(None);
            }
        }
        Ok(Some([self.buf[self.pos], self.buf[self.pos + 1]]))
    }

    // R: gzio.h:check_header, connections.c:gzcon_check_header
    fn check_header(&mut self) -> Header {
        self.started = true;
        let h = match self.peek2() {
            Ok(Some(h)) => h,
            Ok(None) => {
                if self.end > self.pos {
                    self.transparent = true;
                    return Header::Transparent;
                }
                return Header::Eof;
            }
            Err(_) => return Header::Bad,
        };
        if h != [0x1f, 0x8b] {
            self.transparent = true;
            return Header::Transparent;
        }
        self.pos += 2;
        let mut b = || self.byte().ok().flatten();
        let (Some(method), Some(flags)) = (b(), b()) else { return Header::Bad };
        if method != 8 || flags & 0xE0 != 0 {
            return Header::Bad;
        }
        for _ in 0..6 {
            b();
        }
        if flags & 0x04 != 0 {
            let lo = b().unwrap_or(0) as usize;
            let hi = b().unwrap_or(0) as usize;
            for _ in 0..(lo | hi << 8) {
                if b().is_none() {
                    break;
                }
            }
        }
        if flags & 0x08 != 0 {
            while let Some(c) = b() {
                if c == 0 {
                    break;
                }
            }
        }
        if flags & 0x10 != 0 {
            while let Some(c) = b() {
                if c == 0 {
                    break;
                }
            }
        }
        if flags & 0x02 != 0 {
            b();
            b();
        }
        if self.src_eof && self.pos >= self.end {
            return Header::Bad;
        }
        Header::Ok
    }

    /// Read up to `out.len()` bytes, as many as there are.
    fn read_full(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if !self.started {
            match self.check_header() {
                Header::Ok => {}
                Header::Transparent if self.allow => {}
                Header::Transparent | Header::Bad => {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "bad gzip header"));
                }
                Header::Eof => {
                    self.done = true;
                }
            }
        }
        let mut n = 0;
        while n < out.len() && !self.done {
            if self.transparent {
                let k = (self.end - self.pos).min(out.len() - n);
                out[n..n + k].copy_from_slice(&self.buf[self.pos..self.pos + k]);
                self.pos += k;
                n += k;
                if n < out.len() {
                    let k = self.src.read(&mut out[n..])?;
                    if k == 0 {
                        self.done = true;
                    }
                    n += k;
                }
                continue;
            }
            self.fill()?;
            let before_in = self.inflate.total_in();
            let before_out = self.inflate.total_out();
            let st = self
                .inflate
                .decompress(&self.buf[self.pos..self.end], &mut out[n..], FlushDecompress::None)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let used = (self.inflate.total_in() - before_in) as usize;
            let made = (self.inflate.total_out() - before_out) as usize;
            self.pos += used;
            self.crc.update(&out[n..n + made]);
            n += made;
            match st {
                Status::StreamEnd => {
                    // the trailer: CRC-32 and length, little-endian
                    let mut t = [0u8; 8];
                    for x in t.iter_mut() {
                        *x = self.byte()?.unwrap_or(0);
                    }
                    let crc = u32::from_le_bytes([t[0], t[1], t[2], t[3]]);
                    if crc != self.crc.sum() {
                        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "crc error"));
                    }
                    match self.check_header() {
                        Header::Ok => {
                            self.inflate.reset(false);
                            self.crc = Crc::new();
                        }
                        _ => {
                            self.transparent = false;
                            self.done = true;
                        }
                    }
                }
                _ => {
                    if used == 0 && made == 0 && self.src_eof {
                        // truncated
                        self.done = true;
                    }
                }
            }
        }
        Ok(n)
    }
}

/// R_gzwrite: a gzip writer (raw deflate at `level`, the trailer at the end).
struct GzWriter<W: Write> {
    dst: W,
    deflate: Compress,
    crc: Crc,
    out: Vec<u8>,
    header: bool,
}

impl<W: Write> GzWriter<W> {
    fn new(dst: W, level: u32) -> GzWriter<W> {
        GzWriter { header: true, ..GzWriter::without_header(dst, level) }
    }

    fn without_header(dst: W, level: u32) -> GzWriter<W> {
        GzWriter {
            dst,
            deflate: Compress::new(Compression::new(level), false),
            crc: Crc::new(),
            out: Vec::with_capacity(Z_BUFSIZE),
            header: false,
        }
    }

    fn put_header(&mut self) -> std::io::Result<()> {
        if self.header {
            self.header = false;
            self.dst.write_all(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, OS_CODE])?;
        }
        Ok(())
    }

    fn write_data(&mut self, mut data: &[u8]) -> std::io::Result<usize> {
        self.put_header()?;
        let n = data.len();
        self.crc.update(data);
        while !data.is_empty() {
            self.out.clear();
            let before = self.deflate.total_in();
            self.deflate.compress_vec(data, &mut self.out, FlushCompress::None).map_err(std::io::Error::other)?;
            let used = (self.deflate.total_in() - before) as usize;
            data = &data[used..];
            self.dst.write_all(&self.out)?;
            if used == 0 && self.out.is_empty() {
                self.out.reserve(Z_BUFSIZE);
            }
        }
        Ok(n)
    }

    fn finish(mut self) -> std::io::Result<()> {
        self.put_header()?;
        loop {
            self.out.clear();
            self.out.reserve(Z_BUFSIZE);
            let st =
                self.deflate.compress_vec(&[], &mut self.out, FlushCompress::Finish).map_err(std::io::Error::other)?;
            self.dst.write_all(&self.out)?;
            if st == Status::StreamEnd {
                break;
            }
        }
        let mut t = [0u8; 8];
        t[..4].copy_from_slice(&self.crc.sum().to_le_bytes());
        t[4..].copy_from_slice(&(self.deflate.total_in() as u32).to_le_bytes());
        self.dst.write_all(&t)?;
        self.dst.flush()
    }
}

// ---- the table ----------------------------------------------------------------

/// `NextConnection()`: the first free slot from 3, or GNU R's error.
// R: connections.c:NextConnection
fn next_connection(cx: &mut Cx) -> RResult<usize> {
    let free = table(cx).slots.borrow().iter().skip(3).position(|c| c.is_none()).map(|i| i + 3);
    match free {
        Some(i) => Ok(i),
        None => {
            cx.collect();
            let free = table(cx).slots.borrow().iter().skip(3).position(|c| c.is_none()).map(|i| i + 3);
            free.ok_or_else(|| cx.error(&format!("all {NCONNECTIONS} connections are in use")))
        }
    }
}

/// Run `f` on connection `n` (`getConnection(n)`: GNU R's "invalid
/// connection" for a slot that holds none).
// R: connections.c:getConnection
pub fn with_conn<T>(cx: &mut Cx, n: i32, f: impl FnOnce(&mut Conn) -> T) -> RResult<T> {
    let r = {
        let mut t = table(cx).slots.borrow_mut();
        usize::try_from(n).ok().filter(|&i| i < NCONNECTIONS).and_then(|i| t[i].as_deref_mut()).map(f)
    };
    r.ok_or_else(|| cx.error("invalid connection"))
}

/// `R_ReadConnection`'s and the streams' read: exactly `buf.len()` bytes
/// or the error GNU R's caller gives (`msg`).
pub fn read_exact(cx: &mut Cx, n: i32, buf: &mut [u8], msg: &str) -> RResult<()> {
    let r = with_conn(cx, n, |c| {
        if !c.isopen {
            return Err("connection is not open".to_string());
        }
        if !c.canread {
            return Err("cannot read from this connection".to_string());
        }
        match conn_read(c, buf) {
            Ok(k) if k == buf.len() => Ok(()),
            Ok(_) => Err(msg.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::Other => Err(e.to_string()),
            Err(_) => Err(msg.to_string()),
        }
    })?;
    r.map_err(|m| cx.error(&m))
}

/// `InBytesConn`: exactly `buf.len()` bytes from connection `n`
/// (`CheckInConn`'s messages, then "error reading from connection").
// R: serialize.c:InBytesConn, CheckInConn
pub fn read_raw(cx: &Cx, n: i32, buf: &mut [u8]) -> Result<(), String> {
    let mut t = table(cx).slots.borrow_mut();
    let Some(c) = usize::try_from(n).ok().filter(|&i| i < NCONNECTIONS).and_then(|i| t[i].as_deref_mut()) else {
        return Err("invalid connection".into());
    };
    if !c.isopen {
        return Err("connection is not open".into());
    }
    if !c.canread {
        return Err("cannot read from this connection".into());
    }
    match conn_read(c, buf) {
        Ok(k) if k == buf.len() => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::Other => Err(e.to_string()),
        _ => Err("error reading from connection".into()),
    }
}

/// The streams' write: all of `buf`, or `msg`.
pub fn write_all(cx: &mut Cx, n: i32, buf: &[u8], msg: &str) -> RResult<()> {
    let r = with_conn(cx, n, |c| {
        if !c.isopen {
            return Err("connection is not open".to_string());
        }
        if !c.canwrite {
            return Err("cannot write to this connection".to_string());
        }
        match conn_write(c, buf) {
            Ok(k) if k == buf.len() => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::Other => Err(e.to_string()),
            _ => Err(msg.to_string()),
        }
    })?;
    r.map_err(|m| cx.error(&m))
}

/// `con->open(con)` with `mode` in place for the call (serialize's
/// `strcpy(con->mode, "rb")`), then the mode back: false and the warning
/// on failure.
pub fn open_with_mode(cx: &mut Cx, n: i32, mode: &str) -> RResult<bool> {
    let r = with_conn(cx, n, |c| {
        let old = std::mem::replace(&mut c.mode, mode.to_string());
        let r = open_conn(c);
        c.mode = old;
        r
    })?;
    match r {
        Ok(()) => Ok(true),
        Err(w) => {
            cx.warning(&w)?;
            Ok(false)
        }
    }
}

/// `con->close(con)` (not destroying it).
pub fn close_only(cx: &mut Cx, n: i32) -> RResult<()> {
    with_conn(cx, n, |c| {
        close_conn(c);
    })
}

/// A connection's flags the streams check.
#[derive(Clone, Copy, Debug)]
pub struct Flags {
    pub isopen: bool,
    pub canread: bool,
    pub canwrite: bool,
    pub text: bool,
}

pub fn flags(cx: &mut Cx, n: i32) -> RResult<Flags> {
    with_conn(cx, n, |c| Flags { isopen: c.isopen, canread: c.canread, canwrite: c.canwrite, text: c.text })
}

/// `con_destroy(i)`: close it (GNU R's `checkClose` warnings) and free the
/// slot; the status closing gave.
// R: connections.c:con_destroy, con_close1, checkClose
fn con_destroy(cx: &mut Cx, i: usize) -> RResult<i32> {
    let c = table(cx).slots.borrow_mut()[i].take();
    let Some(mut c) = c else { return Err(cx.error("invalid connection")) };
    let err = if c.isopen { close_conn(&mut c) } else { None };
    let status = c.status;
    if let Some(e) = err {
        cx.warning(&format!("Problem closing connection:  {e}"))?;
    }
    if let Some(g) = c.ex_ptr.take() {
        cx.release_global(g);
    }
    if let Imp::Gzcon(g) = &mut c.imp
        && let Some(p) = g.inner.ex_ptr.take()
    {
        cx.release_global(p);
    }
    Ok(status)
}

/// `checked_open(ncon)`: open, or destroy the connection and raise GNU R's
/// error after the open's warning.
// R: connections.c:checked_open
fn checked_open(cx: &mut Cx, ncon: usize) -> RResult<()> {
    let r = with_conn(cx, ncon as i32, open_conn)?;
    if let Err(w) = r {
        // the warning is given while the connection exists (it may become
        // an error, which destroys it, PR#18491)
        let warned = cx.warning(&w);
        con_destroy(cx, ncon)?;
        warned?;
        return Err(cx.error("cannot open the connection"));
    }
    Ok(())
}

/// Install `con` in slot `ncon`, with its id and `conn_id` pointer, and
/// write the connection object (`ScalarInteger(ncon)` of class
/// `c(class, "connection")`) to `out`.
fn publish(cx: &mut Cx, ncon: usize, mut con: Conn, class: &str, out: Out<'_>) -> RResult<()> {
    let t = table(cx);
    let id = t.current_id.get() + 1;
    t.current_id.set(id);
    con.id = id;
    t.slots.borrow_mut()[ncon] = Some(Box::new(con));
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let ptr = o.root();
        cx.build(o, |cx| {
            let tag = cx.install(b"connection")?;
            // SAFETY: an EXTPTRSXP's car is its address (an untraced word),
            // here the connection's id; the tag is a live symbol.
            #[allow(unsafe_code)]
            unsafe {
                let nil = cx.nil().sexp();
                let x = cx.alloc_node_raw(EXTPTRSXP, nil, nil, tag.as_ref().sexp())?;
                x.set_extptr_addr(id as *mut u8);
                Ok(Ref::from_sexp(x, cx))
            }
        })?;
        let g = cx.global(ptr.bind(cx));
        with_conn(cx, ncon as i32, |c| c.ex_ptr = Some(g))?;
        cx.build(out, |cx| {
            let mut v = cx.alloc_copy(&[ncon as i32])?;
            let mut cl = cx.alloc_vector::<Char<'_>>(2)?;
            cl.set(0, cx.mk_char(class.as_bytes(), Encoding::Native)?);
            cl.set(1, cx.mk_char(b"connection", Encoding::Native)?);
            let mut a = Attrs::new();
            a.set_class(cx.sym(RSym::Class), Some(cl.finish().into()));
            a.install(Attr { tag: cx.install(b"conn_id")?, value: ptr.bind(cx) });
            a.apply(cx, &mut v)?;
            Ok(v.finish().into())
        })
    })
}

// ---- the builtins -------------------------------------------------------------

fn string_arg<'g>(x: Ref<'g>) -> Option<Vec<u8>> {
    match x.view() {
        robigo_rtcore::View::Str(v) if v.len() == 1 => Some(v.get(0).bytes().to_vec()),
        _ => None,
    }
}

/// The first element of a string vector of length one that is not NA,
/// translated to the native encoding (`translateCharFP`).
fn description_arg(cx: &mut Cx, x: Root<'_>) -> RResult<Vec<u8>> {
    let r = x.bind(cx);
    let ok = matches!(r.view(), robigo_rtcore::View::Str(v) if v.len() == 1 && v.get(0) != cx.na_string());
    if !ok {
        return Err(cx.error("invalid 'description' argument"));
    }
    let c = r.as_vector::<Char<'_>>().unwrap().get(0);
    match robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale()) {
        Ok(b) => Ok(b.into_owned()),
        Err(_) => Err(cx.error("robigo: a file name not representable in the native encoding is not implemented yet")),
    }
}

fn encoding_arg(cx: &mut Cx, x: Root<'_>) -> RResult<String> {
    match string_arg(x.bind(cx)) {
        Some(b) if b.len() <= 100 => Ok(String::from_utf8_lossy(&b).into_owned()),
        _ => Err(cx.error("invalid 'encoding' argument")),
    }
}

fn check_encoding(cx: &mut Cx, enc: &str) -> RResult<()> {
    if enc.is_empty() || enc == "native.enc" {
        Ok(())
    } else {
        Err(cx.error(&format!("robigo: connections with encoding '{enc}' are not implemented yet")))
    }
}

/// `file(description, open, blocking, encoding, method, raw)`.
// R: connections.c:do_url (PRIMVAL 1)
pub fn do_file(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "file", 6, true)?;
    let url = description_arg(cx, args.root(0))?;
    for scheme in [&b"http://"[..], b"ftp://", b"https://", b"ftps://"] {
        if url.starts_with(scheme) {
            return Err(cx.error("robigo: 'file()' of a URL is not implemented yet"));
        }
    }
    let Some(open) = string_arg(args.get(cx, 1)) else {
        return Err(cx.error("invalid 'open' argument"));
    };
    let mut open = String::from_utf8_lossy(&open).into_owned();
    let block = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 2));
    if block == NA_INTEGER {
        return Err(cx.error("invalid 'blocking' argument"));
    }
    let enc = encoding_arg(cx, args.root(3))?;
    let meth = args.get(cx, 4);
    let cmeth = match meth.view() {
        robigo_rtcore::View::Str(v) if !v.is_empty() => v.get(0).bytes().to_vec(),
        _ => Vec::new(),
    };
    if cmeth == b"wininet" {
        return Err(cx.error("method = \"wininet\" is only supported on Windows"));
    }
    let raw = robigo_rtbuiltins::util::as_bool2(cx, call, args.root(5))?;
    let ncon = next_connection(cx)?;
    let (desc, class) = if let Some(rest) = url.strip_prefix(b"file://") {
        (rest.to_vec(), "file")
    } else {
        if url.is_empty() {
            if open.is_empty() {
                open = "w+".into();
            }
            if open != "w+" && open != "w+b" {
                open = "w+".into();
                cx.warning("file(\"\") only supports open = \"w+\" and open = \"w+b\": using the former")?;
            }
        }
        if matches!(&url[..], b"clipboard" | b"X11_primary" | b"X11_secondary" | b"X11_clipboard") {
            return Err(cx.error("robigo: clipboard connections are not implemented yet"));
        }
        (url.clone(), "file")
    };
    let efn = expand_file_name(&desc);
    if !raw && !desc.is_empty() {
        if let Ok(m) = std::fs::metadata(path_of(&efn)) {
            use std::os::unix::fs::FileTypeExt;
            if m.file_type().is_fifo() {
                let shown = String::from_utf8_lossy(&url).into_owned();
                cx.warning(&format!("using 'raw = TRUE' because '{shown}' is a fifo or pipe"))?;
                return Err(cx.error("robigo: fifo connections are not implemented yet"));
            } else if !m.is_file() && efn != b"/dev/null" {
                let shown = String::from_utf8_lossy(&url).into_owned();
                cx.warning(&format!("'raw = FALSE' but '{shown}' is not a regular file"))?;
            }
        }
    }
    let mode = if open.is_empty() { "r".to_string() } else { open.clone() };
    let mut con = if !raw && (open.is_empty() || open == "r" || open == "rt") && !desc.is_empty() {
        match comp_type_from_file(&efn) {
            0 => new_file(&desc, &mode, raw),
            1 => new_gzfile(&desc, if open.is_empty() { "rt" } else { &open }, 0),
            _ => return Err(cx.error("robigo: 'file()' of a bzip2, xz or zstd file is not implemented yet")),
        }
    } else {
        new_file(&desc, &mode, raw)
    };
    con.blocking = block != 0;
    con.encname = enc.clone();
    if !con.encname.is_empty() && con.encname != "native.enc" {
        con.canseek = false;
    }
    publish(cx, ncon, con, class, out)?;
    if !open.is_empty() {
        check_encoding(cx, &enc)?;
        checked_open(cx, ncon)?;
    }
    Ok(())
}

// R: connections.c:newfile
fn new_file(desc: &[u8], mode: &str, raw: bool) -> Conn {
    let mut c =
        Conn::new("file", desc, mode, Imp::File(FileConn { fp: None, rpos: 0, wpos: 0, last_was_write: false }));
    c.canseek = !raw;
    c
}

// R: connections.c:newgzfile
fn new_gzfile(desc: &[u8], mode: &str, compress: u32) -> Conn {
    let mut c = Conn::new("gzfile", desc, mode, Imp::Gz(GzFile { compress, io: None }));
    c.canseek = true;
    c.text = !mode.contains('b');
    c
}

/// `gzfile(description, open, encoding, compression)` (`bzfile`, `xzfile`
/// and `zstdfile` share it: `CODE`).
// R: connections.c:do_gzfile
pub fn do_gzfile<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let name = ["gzfile", "bzfile", "xzfile", "zstdfile"][CODE as usize];
    crate::prims::check_arity(cx, call, args.len(), name, 4, true)?;
    let file = description_arg(cx, args.root(0))?;
    let Some(open) = string_arg(args.get(cx, 1)) else {
        return Err(cx.error("invalid 'open' argument"));
    };
    let open = String::from_utf8_lossy(&open).into_owned();
    let enc = encoding_arg(cx, args.root(2))?;
    let compress = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(3))?;
    let bad = match CODE {
        0 | 1 => compress == NA_INTEGER || !(0..=9).contains(&compress),
        2 => compress == NA_INTEGER || compress.abs() > 9,
        _ => compress == NA_INTEGER || compress.abs() > 19,
    };
    if bad {
        return Err(cx.error("invalid 'compress' argument"));
    }
    let mut ty = CODE;
    if ty == 0 && (open.is_empty() || open.starts_with('r')) {
        ty = match comp_type_from_file(&expand_file_name(&file)) {
            0 | 1 => 0,
            2 => 1,
            3 => 2,
            _ => 3,
        };
    }
    if ty != 0 {
        return Err(cx.error(&format!(
            "robigo: '{}' connections are not implemented yet",
            ["gzfile", "bzfile", "xzfile", "zstdfile"][ty as usize]
        )));
    }
    let mut con = new_gzfile(&file, if open.is_empty() { "rb" } else { &open }, compress as u32);
    let ncon = next_connection(cx)?;
    con.blocking = true;
    con.encname = enc.clone();
    if !con.encname.is_empty() && con.encname != "native.enc" {
        con.canseek = false;
    }
    publish(cx, ncon, con, "gzfile", out)?;
    if !open.is_empty() {
        if !open.contains('b') {
            check_encoding(cx, &enc)?;
        }
        checked_open(cx, ncon)?;
    }
    Ok(())
}

/// `asInteger(con)` of an object that inherits from "connection", or GNU
/// R's error.
fn conn_number(cx: &mut Cx, x: Root<'_>) -> RResult<i32> {
    if !robigo_rtbuiltins::attrib::inherits(x.bind(cx), b"connection") {
        return Err(cx.error("'con' is not a connection"));
    }
    Ok(robigo_rtbuiltins::coerce::vector::as_integer(cx, x.bind(cx)).0)
}

/// `close(con, type)`.
// R: connections.c:do_close
pub fn do_close(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "close", 2, true)?;
    let i = conn_number(cx, args.root(0))?;
    if (0..3).contains(&i) {
        return Err(cx.error("cannot close standard connections"));
    }
    // (no sinks yet: GNU R's checks against SinkCons and R_ErrorCon find none)
    with_conn(cx, i, |_| ())?;
    let status = con_destroy(cx, i as usize)?;
    if status != NA_INTEGER {
        cx.build_scalar(out, status)
    } else {
        out.set(cx, cx.nil());
        Ok(())
    }
}

/// `gzcon(con, level, allowNonCompressed, text)`.
// R: connections.c:do_gzcon
pub fn do_gzcon(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "gzcon", 4, true)?;
    let icon = conn_number(cx, args.root(0))?;
    with_conn(cx, icon, |_| ())?;
    let level = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(1))?;
    if level == NA_INTEGER || !(0..=9).contains(&level) {
        return Err(cx.error("'level' must be one of 0 ... 9"));
    }
    let allow = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 2));
    if allow == NA_INTEGER {
        return Err(cx.error("'allowNonCompression' must be TRUE or FALSE"));
    }
    let text = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 3));
    if text == NA_INTEGER {
        return Err(cx.error("'text' must be TRUE or FALSE"));
    }
    let (is_gzcon, mode, class) = with_conn(cx, icon, |c| (c.is_gzcon, c.mode.clone(), c.class))?;
    if is_gzcon {
        cx.warning("this is already a 'gzcon' connection")?;
        let x = args.get(cx, 0);
        out.set(cx, x);
        return Ok(());
    }
    let m = mode.as_str();
    let nmode = if m == "r" || m.starts_with("rb") {
        "rb"
    } else if m == "w" || m.starts_with("wb") {
        "wb"
    } else {
        return Err(cx.error("can only use read- or write- binary connections"));
    };
    if class == "file" && (m == "r" || m == "w") {
        cx.warning("using a text-mode 'file' connection may not work correctly")?;
    } else if class == "textConnection" && m == "w" {
        return Err(
            cx.error("cannot create a 'gzcon' connection from a writable textConnection; maybe use rawConnection")
        );
    }
    let inner = table(cx).slots.borrow_mut()[icon as usize].take().expect("checked above");
    let mut desc = b"gzcon(".to_vec();
    desc.extend_from_slice(&inner.description);
    desc.push(b')');
    desc.truncate(999);
    let inner_open = inner.isopen;
    let encname = inner.encname.clone();
    let mut con = Conn::new(
        "gzcon",
        &desc,
        nmode,
        Imp::Gzcon(Box::new(GzconConn { inner, level: level as u32, allow: allow != 0, io: None })),
    );
    con.text = text != 0;
    con.is_gzcon = true;
    con.encname = encname;
    publish(cx, icon as usize, con, "gzcon", out)?;
    if inner_open {
        let r = with_conn(cx, icon, gzcon_open)?;
        if let Err((w, true)) = r {
            cx.warning(&w)?;
        }
    }
    Ok(())
}

/// `summary.connection(object)`: the connection's description, class,
/// mode, text, opened, can read and can write, as strings.
// R: connections.c:do_sumconnection
pub fn do_sumconnection(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "summary.connection", 1, true)?;
    let n = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(0))?;
    let fields = with_conn(cx, n, |c| {
        let yn = |b: bool| if b { b"yes".to_vec() } else { b"no".to_vec() };
        [
            c.description.clone(),
            c.class.as_bytes().to_vec(),
            c.mode.as_bytes().to_vec(),
            if c.text { b"text".to_vec() } else { b"binary".to_vec() },
            if c.isopen { b"opened".to_vec() } else { b"closed".to_vec() },
            yn(c.canread),
            yn(c.canwrite),
        ]
    })?;
    let names: [&[u8]; 7] = [b"description", b"class", b"mode", b"text", b"opened", b"can read", b"can write"];
    cx.build(out, |cx| {
        let mut ans = cx.alloc_vector::<Ref<'_>>(7)?;
        let mut nm = cx.alloc_vector::<Char<'_>>(7)?;
        for i in 0..7 {
            ans.set(i, cx.mk_string(&fields[i], Encoding::Native)?);
            nm.set(i, cx.mk_char(names[i], Encoding::Native)?);
        }
        let mut a = Attrs::new();
        a.install(Attr { tag: cx.sym(RSym::Names), value: nm.finish().into() });
        a.apply(cx, &mut ans)?;
        Ok(ans.finish().into())
    })
}

/// `con->open(con)` in the connection's own mode: false and the warning on
/// failure (do_parse's `if(!con->open(con)) error(...)`).
pub fn open(cx: &mut Cx, n: i32) -> RResult<bool> {
    let r = with_conn(cx, n, open_conn)?;
    match r {
        Ok(()) => Ok(true),
        Err(w) => {
            cx.warning(&w)?;
            Ok(false)
        }
    }
}

/// Everything left to read on connection `n` (`Rconn_fgetc` to the end,
/// as `R_ParseConn` reads it). A read the system fails ends the input, as
/// `fgetc`'s EOF does there; robigo-rt's own refusals (a connection it
/// cannot read yet) are errors.
// R: connections.c:file_fgetc_internal, source.c:R_ParseConn
pub fn read_to_end(cx: &mut Cx, n: i32) -> RResult<Vec<u8>> {
    let r = with_conn(cx, n, |c| {
        let mut all = Vec::new();
        let mut buf = vec![0u8; 65536];
        loop {
            match conn_read(c, &mut buf) {
                Ok(0) => return Ok(all),
                Ok(k) => all.extend_from_slice(&buf[..k]),
                Err(e) if e.raw_os_error().is_none() => return Err(e.to_string()),
                Err(_) => return Ok(all),
            }
        }
    })?;
    r.map_err(|m| cx.error(&m))
}

/// `writeLines(text, con, sep, useBytes)`: each string then `sep`, to the
/// connection (opened in mode "wt" and closed again if it was not open);
/// to the console for `stdout()` and `stderr()`.
// R: connections.c:do_writelines
pub fn do_writelines(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "writeLines", 4, true)?;
    if args.get(cx, 0).sexptype() != STRSXP {
        return Err(cx.error("invalid 'text' argument"));
    }
    let n = conn_number(cx, args.root(1))?;
    with_conn(cx, n, |_| ())?;
    if args.get(cx, 2).sexptype() != STRSXP {
        return Err(cx.error("invalid 'sep' argument"));
    }
    let use_bytes = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 3));
    if use_bytes == NA_INTEGER {
        return Err(cx.error("invalid 'useBytes' argument"));
    }
    // translateChar0 (CHAR for a "bytes" string) or CHAR
    let bytes_of = |cx: &Cx, c: Char<'_>| -> Vec<u8> {
        if c == cx.na_string() {
            return b"NA".to_vec();
        }
        if use_bytes != 0 {
            return c.bytes().to_vec();
        }
        robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale())
            .map(|b| b.into_owned())
            .unwrap_or_else(|_| c.bytes().to_vec())
    };
    let wasopen = flags(cx, n)?.isopen;
    if !wasopen && !open_with_mode(cx, n, "wt")? {
        return Err(cx.error("cannot open the connection"));
    }
    let r = (|| {
        if !flags(cx, n)?.canwrite {
            return Err(cx.error("cannot write to this connection"));
        }
        let altrep = |cx: &mut Cx| {
            robigo_rtbuiltins::util::not_yet(cx, Some(call), "writeLines() of a package ALTREP character vector")
        };
        let mut buf = Vec::new();
        {
            let Some(sep) = args.get(cx, 2).as_vector::<Char<'_>>() else { return Err(altrep(cx)) };
            // STRING_ELT(sep, 0) of an empty vector is R_NilValue, which
            // translateChar0 refuses (CHAR of it, with useBytes, reads past
            // the vector: the same error here)
            if sep.is_empty() {
                return Err(cx.error("'Rf_translateChar0' must be called on a CHARSXP, but got 'NULL'"));
            }
            let sep = bytes_of(cx, sep.get(0));
            let Some(text) = args.get(cx, 0).as_vector::<Char<'_>>() else { return Err(altrep(cx)) };
            for i in 0..text.len() {
                buf.extend_from_slice(&bytes_of(cx, text.get(i)));
                buf.extend_from_slice(&sep);
            }
        }
        // Rconn_printf's result is not looked at: a write the system fails
        // raises nothing (robigo-rt's own refusals do)
        match n {
            1 => cx.rprintf(&buf),
            2 => cx.reprintf(&buf),
            _ => {
                let refused = with_conn(cx, n, |c| match conn_write(c, &buf) {
                    Err(e) if e.raw_os_error().is_none() => Some(e.to_string()),
                    _ => None,
                })?;
                if let Some(m) = refused {
                    return Err(cx.error(&m));
                }
            }
        }
        Ok(())
    })();
    if !wasopen {
        let err = with_conn(cx, n, close_conn)?;
        if let Some(e) = err {
            cx.warning(&format!("Problem closing connection:  {e}"))?;
        }
    }
    r?;
    out.set(cx, cx.nil());
    Ok(())
}

/// Give the last `k` bytes read from a file connection back, so the next
/// read starts there (what `R_ParseConn` leaves unread when it stops after
/// the `n`-th expression). False for any other connection.
pub fn unread(cx: &mut Cx, n: i32, k: usize) -> RResult<bool> {
    with_conn(cx, n, |c| match &mut c.imp {
        Imp::File(f) => match f.fp.as_mut() {
            Some(fp) if !f.last_was_write => fp.seek(SeekFrom::Current(-(k as i64))).is_ok(),
            _ => false,
        },
        _ => false,
    })
}

/// Whether connection `n` is a file connection.
pub fn is_file(cx: &mut Cx, n: i32) -> RResult<bool> {
    with_conn(cx, n, |c| matches!(c.imp, Imp::File(_)))
}
