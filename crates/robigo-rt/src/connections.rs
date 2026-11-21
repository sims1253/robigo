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

use crate::rconn::{CMethods, RconnC};
use flate2::{Compress, Compression, Crc, Decompress, FlushCompress, FlushDecompress, Status};
use robigo_heap::types::*;
use robigo_rtbuiltins::attrib::{Attr, Attrs};
use robigo_rtcore::{Args, Char, Cx, Encoding, Global, Out, RResult, RSym, Ref, Root};
use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char, c_int};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

pub mod readers;
pub mod scan;

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
    /// The C API's methods (robigo-rtapi's entries), once installed: every
    /// connection's `struct Rconn` points at them.
    c_methods: Cell<Option<CMethods>>,
}

impl Connections {
    pub fn new() -> Connections {
        let mut slots: Vec<Option<Box<Conn>>> = (0..NCONNECTIONS).map(|_| None).collect();
        for (i, d) in ["stdin", "stdout", "stderr"].iter().enumerate() {
            let mut c = Conn::new("terminal", d.as_bytes(), if i == 0 { "r" } else { "w" }, Imp::Std);
            c.set_isopen(true);
            c.set_canread(i == 0);
            c.set_canwrite(i != 0);
            c.id = i + 1;
            let mut b = Box::new(c);
            attach_c(&mut b, None);
            slots[i] = Some(b);
        }
        Connections { slots: RefCell::new(slots), current_id: Cell::new(3), c_methods: Cell::new(None) }
    }
}

impl Default for Connections {
    fn default() -> Connections {
        Connections::new()
    }
}

/// The boxed connection's `struct Rconn` made whole for C: `private` is
/// the connection, `id` its id (GNU R's `void *` counter), the methods the
/// C API's (if installed).
fn attach_c(b: &mut Box<Conn>, m: Option<CMethods>) {
    let p: *mut Conn = &mut **b;
    b.c.private = p.cast();
    b.c.id = b.id as *mut std::ffi::c_void;
    if let Some(m) = m {
        b.c.set_methods(&m);
    }
}

/// Install the C API's connection methods (robigo-rtapi's install): every
/// connection's `struct Rconn`, now and later, points at them.
pub fn set_c_methods(cx: &Cx, m: CMethods) {
    let t = table(cx);
    t.c_methods.set(Some(m));
    for b in t.slots.borrow_mut().iter_mut().flatten() {
        b.c.set_methods(&m);
    }
}

fn table(cx: &Cx) -> &Connections {
    cx.ext::<Connections>().expect("robigo: the connection table is not installed (robigo_rt::install)")
}

/// One connection (GNU R's `struct Rconn`, the fields robigo uses).
pub struct Conn {
    /// GNU R's state of the connection (`struct Rconn`, the A5 note's
    /// decision C): C reads and writes it through `Rconnection`, so these
    /// fields are the connection's (the accessors below read them).
    pub c: RconnC,
    pub class: &'static str,
    pub description: Vec<u8>,
    /// `c.class` and `c.description` point into these.
    class_c: CString,
    description_c: CString,
    /// `c.buff` points into this (`buff_init`'s buffer).
    buff: Vec<u8>,
    pub id: usize,
    /// `ex_ptr`: the `conn_id` external pointer.
    ex_ptr: Option<Global>,
    imp: Imp,
    /// `PushBack`'s lines, GNU R's order (the last is the line being read);
    /// `c.push_back` points at `push_back_ptrs`, their addresses, so C
    /// reads `con->PushBack` as GNU R lays it out ([`Conn::sync_push_back`]).
    push_back: Vec<CString>,
    push_back_ptrs: Vec<*mut c_char>,
    /// `set_iconv` would have set up `inconv` or `outconv` (an `encoding`
    /// robigo does not re-encode from yet): reads and writes refuse.
    reencode: bool,
}

/// The flag accessors over `c`'s `Rboolean`s.
macro_rules! rconn_flags {
    ($($get:ident $set:ident;)*) => {
        $(
            #[inline]
            pub fn $get(&self) -> bool {
                self.c.$get != 0
            }
            #[inline]
            pub fn $set(&mut self, v: bool) {
                self.c.$get = v as c_int;
            }
        )*
    };
}

impl Conn {
    rconn_flags! {
        isopen set_isopen;
        canread set_canread;
        canwrite set_canwrite;
        canseek set_canseek;
        text set_text;
        blocking set_blocking;
        is_gzcon set_is_gzcon;
        incomplete set_incomplete;
    }

    /// `status`: what closing returned (NA until a file is closed).
    pub fn status(&self) -> i32 {
        self.c.status
    }

    pub fn set_status(&mut self, s: i32) {
        self.c.status = s;
    }

    /// `mode`: at most 4 characters, as GNU R keeps it.
    pub fn mode(&self) -> String {
        self.c.mode()
    }

    pub fn set_mode(&mut self, m: &str) {
        self.c.set_mode(m);
    }

    pub fn encname(&self) -> String {
        self.c.encname()
    }

    pub fn set_encname(&mut self, e: &str) {
        self.c.set_encname(e);
    }

    /// Point `c.push_back` at the lines and set `c.n_push_back` (the
    /// position is the caller's).
    fn sync_push_back(&mut self) {
        self.push_back_ptrs = self.push_back.iter().map(|l| l.as_ptr().cast_mut()).collect();
        self.c.push_back =
            if self.push_back_ptrs.is_empty() { std::ptr::null_mut() } else { self.push_back_ptrs.as_mut_ptr() };
        self.c.n_push_back = self.push_back.len() as c_int;
    }

    /// The lines C left (`nPushBack` lowered from C, as a package's own
    /// clear would: the lines above it are gone).
    fn push_back_len(&mut self) -> usize {
        let n = self.c.n_push_back.max(0) as usize;
        if n < self.push_back.len() {
            self.push_back.truncate(n);
            self.sync_push_back();
        }
        n
    }

    /// `con_pushback(con, newLine, line)`: `line` up to its first NUL
    /// (`strcpy`), then a newline if `new_line`, on top; read from its
    /// start.
    // R: connections.c:con_pushback
    pub(crate) fn push_back_line(&mut self, line: &[u8], new_line: bool) -> Result<(), String> {
        let n = self.push_back_len();
        if n == c_int::MAX as usize {
            return Err("maximum number of pushback lines exceeded".into());
        }
        let cut = line.iter().position(|&b| b == 0).unwrap_or(line.len());
        let mut l = line[..cut].to_vec();
        if new_line {
            l.push(b'\n');
        }
        self.push_back.push(CString::new(l).expect("cut at the first NUL"));
        self.sync_push_back();
        self.c.pos_push_back = 0;
        Ok(())
    }

    /// The pushback freed (`do_clearpushback`, `con_close1`).
    fn clear_push_back(&mut self) {
        self.push_back.clear();
        self.sync_push_back();
    }

    /// The warnings `R_ExpandFileName` gives in this connection's open
    /// (`file_open`'s and `gzfile_open`'s, before they open anything; a
    /// `gzcon`'s when it opens the wrapped connection). The callback has
    /// no `Cx` to give them, so the open's caller gives them first
    /// ([`run_open`]).
    fn open_warnings(&self) -> Vec<String> {
        match &self.imp {
            Imp::File(_) if !self.description.is_empty() => expand_file_name(&self.description).1,
            Imp::Gz(_) => expand_file_name(&self.description).1,
            Imp::Gzcon(g) if !g.inner.isopen() => g.inner.open_warnings(),
            _ => Vec::new(),
        }
    }
}

enum Imp {
    Std,
    Text(TextConn),
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

/// A text connection's input (`struct textconn`): every string followed
/// by a newline, and the position.
pub(crate) struct TextConn {
    pub(crate) data: Vec<u8>,
    pub(crate) cur: usize,
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
        let class_c = CString::new(class).expect("a class name has no NUL");
        let description_c =
            CString::new(description.iter().copied().filter(|&b| b != 0).collect::<Vec<u8>>()).expect("NULs removed");
        let mut c = Conn {
            c: RconnC::new(mode),
            class,
            description: description.to_vec(),
            class_c,
            description_c,
            buff: Vec::new(),
            id: 0,
            ex_ptr: None,
            imp,
            push_back: Vec::new(),
            push_back_ptrs: Vec::new(),
            reencode: false,
        };
        c.c.class = c.class_c.as_ptr().cast_mut();
        c.c.description = c.description_c.as_ptr().cast_mut();
        c
    }
}

/// The open's failure: the warning GNU R gives before "cannot open the
/// connection".
type OpenResult = Result<(), String>;

/// `R_PATH_MAX`: `PATH_MAX` on Linux.
pub(crate) const R_PATH_MAX: usize = 4096;

/// `R_ExpandFileName_unix(s)` (when readline's expansion left a tilde): a leading `~` or `~user` (up to the first
/// `/`) is that home directory: `HOME` for `~` (the password entry when it
/// is unset or empty), the password entry for `~user`. A name with no such
/// directory is left as it is, and so is one whose expansion would not fit
/// in `R_PATH_MAX`, with the text of GNU R's warning.
// R: sys-unix.c:R_ExpandFileName_unix
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

/// readline's `tilde_expand_word(s)`: `~` and `~/...` take `HOME`, even
/// empty (the password entry of the process's user when it is unset, and
/// nothing when that has none either); `~user` and `~user/...` the user's
/// password entry, or stay as they are.
// R: readline's tilde.c:tilde_expand_word (no hooks: R sets none)
fn tilde_expand_word(s: &[u8]) -> Vec<u8> {
    if s.first() != Some(&b'~') {
        return s.to_vec();
    }
    // isolate_tilde_prefix: what follows the tilde, up to a slash
    let user_len = s[1..].iter().position(|&c| c == b'/').map_or(s.len(), |k| k + 1);
    let home = if user_len == 1 {
        match std::env::var_os("HOME") {
            Some(h) => Some(std::os::unix::ffi::OsStrExt::as_bytes(h.as_os_str()).to_vec()),
            None => robigo_rtcore::clib::passwd_home(None),
        }
        .unwrap_or_default()
    } else {
        match robigo_rtcore::clib::passwd_home(Some(&s[1..user_len])) {
            Some(h) => h,
            None => return s.to_vec(),
        }
    };
    // glue_prefix_and_suffix(home, s, user_len)
    [&home[..], &s[user_len..]].concat()
}

/// `R_ExpandFileName_readline(s)`: [`tilde_expand_word`], cut to
/// `R_PATH_MAX - 1` bytes with the text of GNU R's warning when longer.
// R: sys-std.c:R_ExpandFileName_readline
fn expand_file_name_readline(s: &[u8]) -> (Vec<u8>, Option<String>) {
    let mut v = tilde_expand_word(s);
    if v.len() >= R_PATH_MAX {
        let msg = format!("expanded path length {} would be too long for\n{}\n", v.len(), String::from_utf8_lossy(s));
        v.truncate(R_PATH_MAX - 1);
        return (v, Some(msg));
    }
    (v, None)
}

/// `R_ExpandFileName(s)`: readline's expansion (GNU R's `UsingReadline`
/// is TRUE unless `--no-readline`, which robigo-rt does not take: so in
/// Rscript and `R -f` too), unless what it gives still starts with `~` or
/// `~/`, then [`expand_file_name_unix`]'s.
// R: sys-unix.c:R_ExpandFileName
pub fn expand_file_name(s: &[u8]) -> (Vec<u8>, Vec<String>) {
    let (c, w) = expand_file_name_readline(s);
    let mut ws: Vec<String> = w.into_iter().collect();
    if c.first() != Some(&b'~') || (c.len() > 1 && c[1] != b'/') {
        return (c, ws);
    }
    let (u, w2) = expand_file_name_unix(s);
    ws.extend(w2);
    (u, ws)
}

/// `R_ExpandFileName(name)`, its warning given.
pub fn r_expand_file_name(cx: &mut robigo_rtcore::Cx, name: &[u8]) -> robigo_rtcore::RResult<Vec<u8>> {
    let (v, ws) = expand_file_name(name);
    for w in ws {
        cx.warning(&w)?;
    }
    Ok(v)
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
    let name = expand_file_name(&c.description).0;
    if name == b"stdin" {
        return Err("robigo: 'file(\"stdin\")' is not implemented yet".into());
    }
    let shown = String::from_utf8_lossy(&name).into_owned();
    let fp = match fopen(&name, &c.mode()) {
        Ok(f) => f,
        Err(e) => return Err(format!("cannot open file '{shown}': {}", strerror(&e))),
    };
    if fp.metadata().is_ok_and(|m| m.is_dir()) {
        return Err(format!("cannot open file '{shown}': it is a directory"));
    }
    let mode = c.mode();
    let m = mode.as_bytes();
    let mlen = m.len();
    c.set_isopen(true);
    c.set_canwrite(m[0] == b'w' || m[0] == b'a');
    c.set_canread(!c.canwrite());
    if mlen >= 2 && m[1] == b'+' {
        c.set_canread(true);
        c.set_canwrite(true);
    }
    c.set_text(!(mlen >= 2 && m[mlen - 1] == b'b'));
    // set_buffer: a text connection read from a regular file is buffered
    // (shouldBuffer: fstat's S_ISREG)
    if c.canread() && c.text() && fp.metadata().is_ok_and(|md| md.is_file()) {
        buff_init(c);
    }
    let mut fp = fp;
    let wpos = if c.canwrite() { fp.stream_position().unwrap_or(0) } else { 0 };
    let canread = c.canread();
    if let Imp::File(f) = &mut c.imp {
        f.fp = Some(fp);
        f.rpos = 0;
        f.wpos = wpos;
        f.last_was_write = !canread;
    }
    set_iconv(c);
    Ok(())
}

/// `set_iconv(con)`: no conversion for a binary connection or the native
/// encoding, and `UTF8out` (readLines' and scan's request) dropped; any
/// other encoding is one robigo cannot convert yet ([`Conn::reencode`]).
// R: connections.c:set_iconv
fn set_iconv(c: &mut Conn) {
    let enc = c.encname();
    c.reencode = false;
    if !c.text() || enc.is_empty() || enc == "native.enc" {
        c.c.utf8out = 0;
        return;
    }
    c.reencode = c.canread() || c.canwrite();
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
    let name = expand_file_name(&c.description).0;
    let shown = String::from_utf8_lossy(&name).into_owned();
    if is_dir(&name) {
        return Err(format!("cannot open file '{shown}': it is a directory"));
    }
    let mode = c.mode();
    let write = mode.contains('w') || mode.starts_with('a');
    let Imp::Gz(g) = &mut c.imp else { unreachable!() };
    let io = if write {
        let f = if mode.starts_with('a') { fopen(&name, "ab") } else { fopen(&name, "wb") };
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
    c.set_isopen(true);
    c.set_canwrite(mode.starts_with('w') || mode.starts_with('a'));
    c.set_canread(!c.canwrite());
    c.set_text(!mode.contains('b'));
    set_iconv(c);
    Ok(())
}

// R: connections.c:gzcon_open
fn gzcon_open(c: &mut Conn) -> Result<(), (String, bool)> {
    let Imp::Gzcon(g) = &mut c.imp else { unreachable!() };
    if !g.inner.isopen() {
        open_conn(&mut g.inner).map_err(|m| (m, false))?;
    }
    // (c.c's fields: the gzcon's private part stays borrowed)
    c.c.isopen = 1;
    c.c.canwrite = g.inner.c.canwrite;
    c.c.canread = (c.c.canwrite == 0) as c_int;
    let inner: *mut Conn = &mut *g.inner;
    if c.c.canread != 0 {
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
        // text_open: nothing saved
        Imp::Text(_) => {
            c.c.save = crate::rconn::NO_SAVE;
            Ok(())
        }
        Imp::File(_) => file_open(c),
        Imp::Gz(_) => gzfile_open(c),
        Imp::Gzcon(_) => match gzcon_open(c) {
            Ok(()) => Ok(()),
            Err((m, _)) => Err(m),
        },
    }
}

/// `con->close(con)`, and `con_close1`'s release of the buffer.
fn close_conn(c: &mut Conn) -> Option<String> {
    let mut err = None;
    buff_free(c);
    match &mut c.imp {
        Imp::Std | Imp::Text(_) => {}
        Imp::File(f) => {
            // (c.c's fields: the file's private part stays borrowed)
            // con->status = fclose(this->fp): the writes went to the file as
            // they were made (no stdio buffer), so there is nothing left to
            // flush, and fclose does not sync to the disk; closing it is
            // dropping it (a close(2) error is not reported, as no write is
            // pending)
            if c.c.isopen != 0 {
                drop(f.fp.take());
                c.c.status = 0;
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
            if g.inner.isopen() {
                close_conn(&mut g.inner);
                g.inner.set_isopen(false);
            }
        }
    }
    c.set_isopen(false);
    err
}

/// A read of the connection's stream: the bytes its buffer holds first
/// (`buff_fgetc` filled it; GNU R's text readers all read through it), then
/// `con->read`.
fn conn_read(c: &mut Conn, buf: &mut [u8]) -> std::io::Result<usize> {
    let unread = c.c.buff_stored_len - c.c.buff_pos;
    let k = unread.min(buf.len());
    if k > 0 {
        let from = c.c.buff_pos;
        buf[..k].copy_from_slice(&c.buff[from..from + k]);
        c.c.buff_pos += k;
    }
    if k == buf.len() {
        return Ok(k);
    }
    Ok(k + conn_read_raw(c, &mut buf[k..])?)
}

/// `con->read(buf, 1, n, con)`.
fn conn_read_raw(c: &mut Conn, buf: &mut [u8]) -> std::io::Result<usize> {
    match &mut c.imp {
        Imp::Std => Err(std::io::Error::other("robigo: reading the standard connections is not implemented yet")),
        // null_read
        Imp::Text(_) => Err(std::io::Error::other("'read' not enabled for this connection")),
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
    if c.reencode {
        return Err(std::io::Error::other("robigo: re-encoding a connection is not implemented yet"));
    }
    match &mut c.imp {
        Imp::Std => Err(std::io::Error::other("robigo: writing the standard connections is not implemented yet")),
        // null_write
        Imp::Text(_) => Err(std::io::Error::other("'write' not enabled for this connection")),
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

// ---- the buffer and the C view (getConnection, Rconn_fgetc, the methods) ----

/// `RBUFFCON_LEN_DEFAULT`.
const RBUFFCON_LEN_DEFAULT: usize = 4096;

/// `R_EOF`.
const R_EOF: c_int = -1;

/// `buff_init(con)`: an empty buffer of the default length.
// R: connections.c:buff_init, buff_set_len
fn buff_init(c: &mut Conn) {
    c.buff = vec![0; RBUFFCON_LEN_DEFAULT];
    c.c.buff = c.buff.as_mut_ptr();
    c.c.buff_len = RBUFFCON_LEN_DEFAULT;
    c.c.buff_pos = 0;
    c.c.buff_stored_len = 0;
}

/// The buffer released (`con_close1`).
// R: connections.c:con_close1
fn buff_free(c: &mut Conn) {
    c.buff = Vec::new();
    c.c.buff = std::ptr::null_mut();
    c.c.buff_len = 0;
    c.c.buff_pos = 0;
    c.c.buff_stored_len = 0;
}

/// `buff_fill(con)`: the unread bytes moved to the front, the rest filled
/// by `con->read`; how many it read.
// R: connections.c:buff_fill, buff_reset
fn buff_fill(c: &mut Conn) -> std::io::Result<usize> {
    let (pos, stored, len) = (c.c.buff_pos, c.c.buff_stored_len, c.c.buff_len);
    c.buff.copy_within(pos..stored, 0);
    c.c.buff_pos = 0;
    c.c.buff_stored_len = stored - pos;
    let mut tmp = std::mem::take(&mut c.buff);
    let r = conn_read_raw(c, &mut tmp[stored - pos..len]);
    c.buff = tmp;
    c.c.buff = c.buff.as_mut_ptr();
    let n = r?;
    c.c.buff_stored_len += n;
    Ok(n)
}

/// `buff_fgetc(con)`.
// R: connections.c:buff_fgetc
fn buff_fgetc(c: &mut Conn) -> Result<c_int, String> {
    if c.c.buff_stored_len == c.c.buff_pos {
        match buff_fill(c) {
            Ok(0) => return Ok(R_EOF),
            Ok(_) => {}
            Err(_) => return Err("error reading from the connection".to_owned()),
        }
    }
    let v = c.buff[c.c.buff_pos];
    c.c.buff_pos += 1;
    Ok(v as c_int)
}

/// `con->fgetc_internal(con)`: one byte of the stream, `R_EOF` at its end.
// R: connections.c:file_fgetc_internal (and the other classes')
fn fgetc_internal(c: &mut Conn) -> Result<c_int, String> {
    let mut b = [0u8; 1];
    match conn_read_raw(c, &mut b) {
        Ok(0) => Ok(R_EOF),
        Ok(_) => Ok(b[0] as c_int),
        Err(e) => Err(e.to_string()),
    }
}

/// `con->fgetc(con)`, `dummy_fgetc`: through the buffer when there is one,
/// else `fgetc_internal`. robigo has no re-encoding (`inconv` is never set).
// R: connections.c:dummy_fgetc
fn dummy_fgetc(c: &mut Conn) -> Result<c_int, String> {
    // a text connection's fgetc is text_fgetc: the byte as C's (signed)
    // char, so 0xFF reads as R_EOF
    // R: connections.c:text_fgetc
    if let Imp::Text(t) = &mut c.imp {
        return Ok(match t.data.get(t.cur) {
            Some(&b) => {
                t.cur += 1;
                b as i8 as c_int
            }
            None => R_EOF,
        });
    }
    if !c.c.inconv.is_null() || c.reencode {
        return Err("robigo: re-encoding a connection is not implemented yet".to_owned());
    }
    if !c.c.buff.is_null() { buff_fgetc(c) } else { fgetc_internal(c) }
}

/// `R_IsNA(x)`: NA, not another NaN (the low word 1954).
// R: arithmetic.c:R_IsNA
fn r_is_na(x: f64) -> bool {
    x.is_nan() && (x.to_bits() & 0xffff_ffff) == 1954
}

/// `con->seek(con, where, origin, rw)` of a file: the position before
/// (the read position for `rw` 1, the write one for 2), then the move
/// (none for `where` NA; a move the system refuses, as to a negative
/// offset, leaves the position).
// R: connections.c:file_seek
fn file_seek(c: &mut Conn, where_: f64, origin: c_int, rw: c_int) -> Result<f64, String> {
    let (canread, canwrite) = (c.canread(), c.canwrite());
    let Imp::File(f) = &mut c.imp else {
        return Err(format!("robigo: seeking a '{}' connection is not implemented yet", c.class));
    };
    let Some(fp) = f.fp.as_mut() else { return Err("connection is not open".into()) };
    // make sure both positions are set
    let mut pos = fp.stream_position().unwrap_or(0);
    if f.last_was_write {
        f.wpos = pos;
    } else {
        f.rpos = pos;
    }
    if rw == 1 {
        if !canread {
            return Err("connection is not open for reading".into());
        }
        pos = f.rpos;
        f.last_was_write = false;
    }
    if rw == 2 {
        if !canwrite {
            return Err("connection is not open for writing".into());
        }
        pos = f.wpos;
        f.last_was_write = true;
    }
    if r_is_na(where_) {
        return Ok(pos as f64);
    }
    let w = where_ as i64;
    let to = match origin {
        2 => SeekFrom::Current(w),
        3 => SeekFrom::End(w),
        _ => {
            if w < 0 {
                // fseeko's EINVAL: no move
                SeekFrom::Current(0)
            } else {
                SeekFrom::Start(w as u64)
            }
        }
    };
    let _ = fp.seek(to);
    let now = fp.stream_position().unwrap_or(0);
    if f.last_was_write {
        f.wpos = now;
    } else {
        f.rpos = now;
    }
    Ok(pos as f64)
}

/// `Rconn_seek(con, where, origin, rw)`: through the buffer when there is
/// one (a read's seek drops what it holds unread, unless it moves within
/// it from the current position).
// R: connections.c:Rconn_seek, buff_seek
pub(crate) fn rconn_seek(c: &mut Conn, mut where_: f64, origin: c_int, rw: c_int) -> Result<f64, String> {
    if c.c.buff.is_null() || rw == 2 {
        return file_seek(c, where_, origin, rw);
    }
    let unread_len = (c.c.buff_stored_len - c.c.buff_pos) as f64;
    if r_is_na(where_) {
        // tell
        return Ok(file_seek(c, where_, origin, rw)? - unread_len);
    }
    if origin == 2 {
        // current
        if where_ < unread_len {
            c.c.buff_pos = (c.c.buff_pos as f64 + where_) as usize;
            return file_seek(c, robigo_heap::NA_REAL, origin, rw);
        }
        where_ -= unread_len;
    }
    c.c.buff_pos = 0;
    c.c.buff_stored_len = 0;
    file_seek(c, where_, origin, rw)
}

/// The slot of the connection whose `struct Rconn` C holds at `con`.
fn slot_of(cx: &Cx, con: *const RconnC) -> Option<i32> {
    table(cx)
        .slots
        .borrow()
        .iter()
        .position(|s| s.as_deref().is_some_and(|c| std::ptr::eq(&c.c, con)))
        .map(|i| i as i32)
}

/// [`with_conn`] by the address C holds: a pointer that is no connection of
/// the isolate is not one robigo made.
fn with_c<T>(cx: &mut Cx, con: *mut RconnC, f: impl FnOnce(&mut Conn) -> Result<T, String>) -> RResult<T> {
    let Some(n) = slot_of(cx, con) else {
        return Err(cx.error("robigo: a connection robigo did not make is not implemented yet"));
    };
    match with_conn(cx, n, f)? {
        Ok(v) => Ok(v),
        Err(m) => Err(cx.error(&m)),
    }
}

/// `getConnection(n)`: the connection's `struct Rconn`.
// R: connections.c:getConnection
pub fn get_connection(cx: &mut Cx, n: i32) -> RResult<*mut RconnC> {
    with_conn(cx, n, |c| &mut c.c as *mut RconnC)
}

/// `con->open(con)`: true, or false after the warning GNU R's open gives.
// R: connections.c:file_open, gzfile_open, gzcon_open
pub fn c_open(cx: &mut Cx, con: *mut RconnC) -> RResult<bool> {
    let Some(n) = slot_of(cx, con) else {
        return Err(cx.error("robigo: a connection robigo did not make is not implemented yet"));
    };
    match run_open(cx, n, open_conn)? {
        Ok(()) => Ok(true),
        Err(w) => {
            cx.warning(&w)?;
            Ok(false)
        }
    }
}

/// `con->close(con)`.
// R: connections.c:file_close, gzfile_close, gzcon_close
pub fn c_close(cx: &mut Cx, con: *mut RconnC) -> RResult<()> {
    with_c(cx, con, |c| {
        close_conn(c);
        Ok(())
    })
}

/// `con->fgetc(con)` (`dummy_fgetc`).
pub fn c_fgetc(cx: &mut Cx, con: *mut RconnC) -> RResult<c_int> {
    with_c(cx, con, dummy_fgetc)
}

/// `con->fgetc_internal(con)`.
pub fn c_fgetc_internal(cx: &mut Cx, con: *mut RconnC) -> RResult<c_int> {
    with_c(cx, con, fgetc_internal)
}

/// `con->read(ptr, size, nitems, con)`: the items read whole.
// R: connections.c:file_read (fread)
pub fn c_read(cx: &mut Cx, buf: &mut [u8], size: usize, con: *mut RconnC) -> RResult<usize> {
    with_c(cx, con, |c| conn_read_raw(c, buf).map(|n| n.checked_div(size).unwrap_or(0)).map_err(|e| e.to_string()))
}

/// `con->write(ptr, size, nitems, con)`: the items written.
// R: connections.c:file_write (fwrite)
pub fn c_write(cx: &mut Cx, buf: &[u8], size: usize, con: *mut RconnC) -> RResult<usize> {
    with_c(cx, con, |c| conn_write(c, buf).map(|n| n.checked_div(size).unwrap_or(0)).map_err(|e| e.to_string()))
}

/// `con->fflush(con)`: a file's stream has no buffer of its own here (0,
/// fflush's success); the other classes' are not implemented yet.
// R: connections.c:file_fflush
pub fn c_fflush(cx: &mut Cx, con: *mut RconnC) -> RResult<c_int> {
    with_c(cx, con, |c| match c.imp {
        Imp::File(_) => Ok(0),
        _ => Err(format!("robigo: fflush of a '{}' connection is not implemented yet", c.class)),
    })
}

/// `Rconn_fgetc(con)`, C's entry.
pub fn rconn_fgetc(cx: &mut Cx, con: *mut RconnC) -> RResult<c_int> {
    let ours = our_fgetc(cx);
    with_c(cx, con, |c| conn_fgetc(c, ours))
}

/// The address of the C API's `fgetc` method (none before it is installed,
/// as a connection's own `c.fgetc` is then): [`conn_fgetc`]'s check that C
/// did not replace a connection's method.
fn our_fgetc(cx: &Cx) -> Option<usize> {
    table(cx).c_methods.get().map(|m| m.fgetc as usize)
}

/// `Rconn_fgetc(con)`: a character saved by a look-ahead, else the
/// pushback's (a line popped at its end), else the stream's next, a CR or
/// CRLF read as LF. `ours` is [`our_fgetc`]: a `fgetc` method C replaced is
/// not robigo's.
// R: connections.c:Rconn_fgetc
pub(crate) fn conn_fgetc(c: &mut Conn, ours: Option<usize>) -> Result<c_int, String> {
    if c.c.save2 != crate::rconn::NO_SAVE {
        let v = c.c.save2;
        c.c.save2 = crate::rconn::NO_SAVE;
        return Ok(v);
    }
    let n = c.push_back_len();
    if n == 0 {
        // map CR or CRLF to LF
        if c.c.save != crate::rconn::NO_SAVE {
            let v = c.c.save;
            c.c.save = crate::rconn::NO_SAVE;
            return Ok(v);
        }
        if c.c.fgetc.map(|f| f as usize) != ours {
            return Err("robigo: a connection whose fgetc C replaced is not implemented yet".to_owned());
        }
        let mut v = dummy_fgetc(c)?;
        if v == b'\r' as c_int {
            v = dummy_fgetc(c)?;
            if v != b'\n' as c_int {
                c.c.save = if v != b'\r' as c_int { v } else { b'\n' as c_int };
                return Ok(b'\n' as c_int);
            }
        }
        return Ok(v);
    }
    let line = c.push_back[n - 1].as_bytes_with_nul();
    let pos = c.c.pos_push_back.max(0) as usize;
    let Some(&b) = line.get(pos) else {
        return Err("robigo: a pushback position C set past its line is not implemented yet".to_owned());
    };
    c.c.pos_push_back += 1;
    if pos + 1 >= line.len() - 1 {
        // last character on a line, so pop the line
        c.push_back.pop();
        c.sync_push_back();
        c.c.pos_push_back = 0;
    }
    Ok(b as c_int)
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
/// and stops at trailing data that is not a member. When `seek` and `tell`
/// on gz connections are ported: a seek drops the block buffer (`obuf`,
/// `opos`, `oend`, `ahead`, `failed`), and `tell` is a position counted
/// across members and transparent reads (gzio.h's `s->out`, which
/// `R_gztell` returns), less the bytes not yet read from the buffer
/// (`oend - opos`): not the inflater's `total_out`, which a new member
/// resets and a transparent read never moves.
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
    /// Inflated bytes not yet read, `obuf[opos..oend]`: a small read is
    /// served from here, and the stream inflated a `Z_BUFSIZE` block at a
    /// time (gzio.h reads its input through a buffer of that size too).
    obuf: Vec<u8>,
    opos: usize,
    oend: usize,
    /// An error found while inflating ahead ([`Ahead`]): raised by the read
    /// that would have met it inflating in place, as before the buffer.
    ahead: Option<Ahead>,
    /// The error a read met: every later read meets one again, as the
    /// stream left in its error state made them before the buffer. A bad
    /// deflate stream's later reads ask the inflater again (its message
    /// for a repeated call); a trailer's error is given again as it was.
    failed: Option<(std::io::ErrorKind, String, bool)>,
}

/// An error a [`GzReader`] found inflating a block ahead of the reads: the
/// block holds the bytes inflated before it, and `oend` is its boundary,
/// which the read that reaches it meets: a member's trailer is checked when
/// its last byte comes out, and inflate goes on to the next block header or
/// symbol, and so to a bad one, with the output full.
struct Ahead {
    err: std::io::Error,
    /// A bad deflate stream (else a member's trailer).
    deflate: bool,
}

/// What one [`GzReader::inflate_into`] step made: the bytes, and an error
/// met after them.
struct Step {
    made: usize,
    err: Option<Ahead>,
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
            obuf: Vec::new(),
            opos: 0,
            oend: 0,
            ahead: None,
            failed: None,
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

    /// Record `e` as the error every later read meets ([`GzReader::failed`]).
    fn fail(&mut self, e: std::io::Error, deflate: bool) -> std::io::Error {
        self.failed = Some((e.kind(), e.to_string(), deflate));
        e
    }

    /// Read up to `out.len()` bytes, as many as there are.
    fn read_full(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if let Some((kind, msg, deflate)) = &self.failed {
            if *deflate {
                // (the inflater's own answer to a call after its error)
                let mut one = [0u8; 1];
                if let Err(e) = self.inflate.decompress(&[], &mut one, FlushDecompress::None) {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                }
            }
            return Err(std::io::Error::new(*kind, msg.clone()));
        }
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
        let mut n = self.from_block(out, 0)?;
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
            if out.len() - n >= Z_BUFSIZE {
                // a large read: inflated in place, an error met at once
                let st = self.inflate_into(&mut out[n..])?;
                if let Some(a) = st.err {
                    return Err(self.fail(a.err, a.deflate));
                }
                n += st.made;
            } else {
                // a small one: a block into the buffer, then from there
                let mut ob = std::mem::take(&mut self.obuf);
                ob.resize(Z_BUFSIZE, 0);
                let st = self.inflate_into(&mut ob);
                self.obuf = ob;
                let st = st?;
                (self.opos, self.oend, self.ahead) = (0, st.made, st.err);
                n = self.from_block(out, n)?;
            }
        }
        Ok(n)
    }

    /// Copy what the block holds into `out[n..]`; the new count. An error
    /// found ahead ([`Ahead`]) is raised, with nothing copied, by the read
    /// that would have met it inflating in place: one that reaches the
    /// block's boundary.
    fn from_block(&mut self, out: &mut [u8], n: usize) -> std::io::Result<usize> {
        let left = self.oend - self.opos;
        let want = out.len() - n;
        if self.ahead.is_some() && want > 0 && want >= left {
            let a = self.ahead.take().expect("robigo: an error ahead");
            self.opos = self.oend;
            return Err(self.fail(a.err, a.deflate));
        }
        let k = left.min(want);
        out[n..n + k].copy_from_slice(&self.obuf[self.opos..self.opos + k]);
        self.opos += k;
        Ok(n + k)
    }

    /// One step of the stream into `dst`: inflate what the input buffer
    /// holds (refilled when empty), the CRC-32 over what came out; at a
    /// member's end its trailer checked and the next member's header read.
    /// What came out (0 at the end, `done` then set), and an error met after
    /// it: a bad deflate stream, or a member's trailer that does not match
    /// (`done` set). An error of the source is returned as such.
    fn inflate_into(&mut self, dst: &mut [u8]) -> std::io::Result<Step> {
        self.fill()?;
        let before_in = self.inflate.total_in();
        let before_out = self.inflate.total_out();
        let r = self.inflate.decompress(&self.buf[self.pos..self.end], dst, FlushDecompress::None);
        let used = (self.inflate.total_in() - before_in) as usize;
        let made = (self.inflate.total_out() - before_out) as usize;
        self.pos += used;
        self.crc.update(&dst[..made]);
        let st = match r {
            Ok(st) => st,
            Err(e) => {
                self.done = true;
                let err = std::io::Error::new(std::io::ErrorKind::InvalidData, e);
                return Ok(Step { made, err: Some(Ahead { err, deflate: true }) });
            }
        };
        match st {
            Status::StreamEnd => {
                // the trailer: CRC-32 and length, little-endian
                let mut t = [0u8; 8];
                for x in t.iter_mut() {
                    *x = self.byte()?.unwrap_or(0);
                }
                let crc = u32::from_le_bytes([t[0], t[1], t[2], t[3]]);
                if crc != self.crc.sum() {
                    self.done = true;
                    let err = std::io::Error::new(std::io::ErrorKind::InvalidData, "crc error");
                    return Ok(Step { made, err: Some(Ahead { err, deflate: false }) });
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
        Ok(Step { made, err: None })
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
        if !c.isopen() {
            return Err("connection is not open".to_string());
        }
        if !c.canread() {
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

/// `con->read(buf, 1, buf.len(), con)`: as many bytes as there are, up
/// to `buf.len()` (robigo-rt's own refusals are errors; a read the system
/// fails reads nothing more).
pub fn read_upto(cx: &mut Cx, n: i32, buf: &mut [u8]) -> RResult<usize> {
    let r = with_conn(cx, n, |c| match conn_read(c, buf) {
        Ok(k) => Ok(k),
        Err(e) if e.raw_os_error().is_none() => Err(e.to_string()),
        Err(_) => Ok(0),
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
    if !c.isopen() {
        return Err("connection is not open".into());
    }
    if !c.canread() {
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
        if !c.isopen() {
            return Err("connection is not open".to_string());
        }
        if !c.canwrite() {
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

/// Run an open `f` on connection `n`, after the warnings `R_ExpandFileName`
/// gives in it (GNU R gives them inside the open, before it opens the file,
/// so before the open's own warning, which is the caller's).
fn run_open(cx: &mut Cx, n: i32, f: impl FnOnce(&mut Conn) -> OpenResult) -> RResult<OpenResult> {
    let ws = with_conn(cx, n, |c| c.open_warnings())?;
    for w in ws {
        cx.warning(&w)?;
    }
    with_conn(cx, n, f)
}

/// `con->open(con)` with `mode` in place for the call (serialize's
/// `strcpy(con->mode, "rb")`), then the mode back: false and the warning
/// on failure.
pub fn open_with_mode(cx: &mut Cx, n: i32, mode: &str) -> RResult<bool> {
    let r = run_open(cx, n, |c| {
        let old = c.mode();
        c.set_mode(mode);
        let r = open_conn(c);
        c.set_mode(&old);
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
    with_conn(cx, n, |c| Flags { isopen: c.isopen(), canread: c.canread(), canwrite: c.canwrite(), text: c.text() })
}

/// `con_destroy(i)`: close it (GNU R's `checkClose` warnings) and free the
/// slot; the status closing gave.
// R: connections.c:con_destroy, con_close1, checkClose
fn con_destroy(cx: &mut Cx, i: usize) -> RResult<i32> {
    let c = table(cx).slots.borrow_mut()[i].take();
    let Some(mut c) = c else { return Err(cx.error("invalid connection")) };
    let err = if c.isopen() { close_conn(&mut c) } else { None };
    let status = c.status();
    c.clear_push_back();
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
    // the open's warnings are given while the connection exists; one that
    // becomes an error destroys it (the context's cend, PR#18491), before
    // the file is opened when it is R_ExpandFileName's
    let ws = with_conn(cx, ncon as i32, |c| c.open_warnings())?;
    let mut warned = Ok(());
    for w in &ws {
        warned = cx.warning(w);
        if warned.is_err() {
            break;
        }
    }
    let r = if warned.is_ok() { with_conn(cx, ncon as i32, open_conn)? } else { Ok(()) };
    if warned.is_ok()
        && let Err(w) = &r
    {
        warned = cx.warning(w);
    }
    if warned.is_err() || r.is_err() {
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
    let mut b = Box::new(con);
    attach_c(&mut b, t.c_methods.get());
    t.slots.borrow_mut()[ncon] = Some(b);
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
    let efn = r_expand_file_name(cx, &desc)?;
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
    con.set_blocking(block != 0);
    con.set_encname(&enc.clone());
    if !con.encname().is_empty() && con.encname() != "native.enc" {
        con.set_canseek(false);
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
    c.set_canseek(!raw);
    c
}

// R: connections.c:newgzfile
fn new_gzfile(desc: &[u8], mode: &str, compress: u32) -> Conn {
    let mut c = Conn::new("gzfile", desc, mode, Imp::Gz(GzFile { compress, io: None }));
    c.set_canseek(true);
    c.set_text(!mode.contains('b'));
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
        ty = match comp_type_from_file(&r_expand_file_name(cx, &file)?) {
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
    con.set_blocking(true);
    con.set_encname(&enc.clone());
    if !con.encname().is_empty() && con.encname() != "native.enc" {
        con.set_canseek(false);
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
    let (is_gzcon, mode, class) = with_conn(cx, icon, |c| (c.is_gzcon(), c.mode().clone(), c.class))?;
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
    let inner_open = inner.isopen();
    let encname = inner.encname().clone();
    let mut con = Conn::new(
        "gzcon",
        &desc,
        nmode,
        Imp::Gzcon(Box::new(GzconConn { inner, level: level as u32, allow: allow != 0, io: None })),
    );
    con.set_text(text != 0);
    con.set_is_gzcon(true);
    con.set_encname(&encname);
    publish(cx, icon as usize, con, "gzcon", out)?;
    if inner_open {
        let ws = with_conn(cx, icon, |c| c.open_warnings())?;
        for w in ws {
            cx.warning(&w)?;
        }
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
            c.mode().as_bytes().to_vec(),
            if c.text() { b"text".to_vec() } else { b"binary".to_vec() },
            if c.isopen() { b"opened".to_vec() } else { b"closed".to_vec() },
            yn(c.canread()),
            yn(c.canwrite()),
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
    let r = run_open(cx, n, open_conn)?;
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
    let ours = our_fgetc(cx);
    let r = with_conn(cx, n, |c| {
        let mut all = Vec::new();
        // what Rconn_fgetc gives before the stream: a saved character, the
        // pushback; a text connection has no stream but its fgetc
        while c.c.save2 != crate::rconn::NO_SAVE
            || c.push_back_len() > 0
            || c.c.save != crate::rconn::NO_SAVE
            || matches!(c.imp, Imp::Text(_))
        {
            match conn_fgetc(c, ours)? {
                R_EOF => return Ok(all),
                ch => all.push(ch as u8),
            }
        }
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

/// Whether connection `n` has input Rconn_fgetc gives before its stream
/// (a saved character, the pushback).
pub fn has_pending_input(cx: &mut Cx, n: i32) -> RResult<bool> {
    with_conn(cx, n, |c| {
        c.c.save2 != crate::rconn::NO_SAVE || c.push_back_len() > 0 || c.c.save != crate::rconn::NO_SAVE
    })
}

/// Whether connection `n` is a file connection.
pub fn is_file(cx: &mut Cx, n: i32) -> RResult<bool> {
    with_conn(cx, n, |c| matches!(c.imp, Imp::File(_)))
}

#[cfg(test)]
mod expand_tests {
    use super::{R_PATH_MAX, expand_file_name, expand_file_name_unix, tilde_expand_word};
    use robigo_rtcore::clib::passwd_home;

    #[test]
    fn names_without_a_tilde_or_a_known_user_stay() {
        for s in [&b"etc/x"[..], b"", b"a~b", b"/~root", b"~robigo-no-such-user/x", b"~robigo-no-such-user", b"~a\0b/x"]
        {
            assert_eq!(expand_file_name_unix(s), (s.to_vec(), None), "{s:?}");
        }
    }

    #[test]
    fn tilde_user_is_split_at_the_first_slash() {
        // (root's entry, where the password database has one)
        let Some(home) = passwd_home(Some(b"root")) else { return };
        let with = |tail: &[u8]| [&home[..], tail].concat();
        assert_eq!(expand_file_name_unix(b"~root"), (home.clone(), None));
        assert_eq!(expand_file_name_unix(b"~root/"), (with(b"/"), None));
        assert_eq!(expand_file_name_unix(b"~root/x/y"), (with(b"/x/y"), None));
        assert_eq!(expand_file_name_unix(b"~root//x"), (with(b"//x"), None));
    }

    #[test]
    fn tilde_alone_is_home_else_the_process_user() {
        let home = match std::env::var_os("HOME") {
            Some(h) if !h.is_empty() => Some(std::os::unix::ffi::OsStrExt::as_bytes(h.as_os_str()).to_vec()),
            _ => passwd_home(None),
        };
        let Some(home) = home else {
            assert_eq!(expand_file_name_unix(b"~/x"), (b"~/x".to_vec(), None));
            return;
        };
        assert_eq!(expand_file_name_unix(b"~"), (home.clone(), None));
        assert_eq!(expand_file_name_unix(b"~/x"), ([&home[..], b"/x"].concat(), None));
    }

    #[test]
    fn an_expansion_from_r_path_max_on_is_refused_with_the_warning() {
        let Some(home) = passwd_home(Some(b"root")) else { return };
        // home + "/" + tail: one byte under the limit fits, the limit does not
        let fits = R_PATH_MAX - 1 - home.len() - 1;
        let s = [&b"~root/"[..], &vec![b'a'; fits]].concat();
        let (v, w) = expand_file_name_unix(&s);
        assert_eq!((v.len(), w), (R_PATH_MAX - 1, None));
        let s = [&b"~root/"[..], &vec![b'a'; fits + 1]].concat();
        let msg = format!("expanded path length {R_PATH_MAX} would be too long for\n{}\n", String::from_utf8_lossy(&s));
        assert_eq!(expand_file_name_unix(&s), (s.clone(), Some(msg)));
    }

    #[test]
    fn readline_expansion_keeps_unknown_users_and_cuts_long_names() {
        for s in [&b"etc/x"[..], b"", b"a~b", b"/~root", b"~robigo-no-such-user/x", b"~robigo-no-such-user"] {
            assert_eq!(tilde_expand_word(s), s.to_vec(), "{s:?}");
            assert_eq!(expand_file_name(s), (s.to_vec(), vec![]), "{s:?}");
        }
        // any name of R_PATH_MAX bytes or more, with or without a tilde
        let s = vec![b'a'; R_PATH_MAX];
        let (v, w) = expand_file_name(&s);
        assert_eq!(v, s[..R_PATH_MAX - 1].to_vec());
        let msg = format!("expanded path length {R_PATH_MAX} would be too long for\n{}\n", String::from_utf8_lossy(&s));
        assert_eq!(w, vec![msg]);
        let s = vec![b'a'; R_PATH_MAX - 1];
        assert_eq!(expand_file_name(&s), (s.clone(), vec![]));
        let Some(home) = passwd_home(Some(b"root")) else { return };
        let s = [&b"~root/"[..], &vec![b'b'; R_PATH_MAX]].concat();
        let (v, w) = expand_file_name(&s);
        assert_eq!(v.len(), R_PATH_MAX - 1);
        assert!(v.starts_with(&home));
        assert_eq!(w.len(), 1);
        assert_eq!(expand_file_name(b"~root/x").0, [&home[..], b"/x"].concat());
    }
}

#[cfg(test)]
mod gz_reader_tests {
    //! `GzReader`'s output buffer: the bytes are those of reading in
    //! place, and an error found inflating ahead is met by the read that
    //! would have met it in place (the read reaching its boundary), then by
    //! every later read.
    use super::{GzReader, Z_BUFSIZE};
    use std::io::{Cursor, Write};

    fn gz(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    /// A member of one stored, non-final block holding `data`, then an
    /// invalid block type.
    fn stored_then_bad(data: &[u8]) -> Vec<u8> {
        let mut v = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3, 0];
        v.extend_from_slice(&(data.len() as u16).to_le_bytes());
        v.extend_from_slice(&(!(data.len() as u16)).to_le_bytes());
        v.extend_from_slice(data);
        v.push(0x07);
        v.extend_from_slice(&[0; 16]);
        v
    }

    fn reader(bytes: Vec<u8>) -> GzReader<Cursor<Vec<u8>>> {
        GzReader::new(Cursor::new(bytes), true)
    }

    #[test]
    fn small_and_large_reads_give_the_data() {
        let data: Vec<u8> = (0..3 * Z_BUFSIZE as u32 + 17).map(|i| (i * 7 % 251) as u8).collect();
        for pattern in [&[1usize, 3, 4][..], &[4], &[Z_BUFSIZE], &[5, Z_BUFSIZE + 3, 2]] {
            let mut r = reader([gz(&data[..1000]), gz(&data[1000..])].concat());
            let mut got = Vec::new();
            let mut i = 0;
            loop {
                let mut b = vec![0; pattern[i % pattern.len()]];
                i += 1;
                let n = r.read_full(&mut b).unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&b[..n]);
            }
            assert_eq!(got, data, "{pattern:?}");
        }
    }

    #[test]
    fn a_bad_crc_is_met_by_the_read_reaching_the_end_and_sticks() {
        let mut bytes = gz(b"abcdef");
        let k = bytes.len() - 8;
        bytes[k] ^= 0xff;
        let mut r = reader(bytes.clone());
        let mut b = [0u8; 4];
        assert_eq!(r.read_full(&mut b).unwrap(), 4);
        assert!(r.read_full(&mut b[..2]).is_err(), "the read reaching the end");
        assert!(r.read_full(&mut b).is_err(), "and every later one");
        let mut r = reader(bytes);
        assert!(r.read_full(&mut [0u8; 6]).is_err());
    }

    #[test]
    fn a_bad_block_is_met_at_its_boundary_not_before() {
        let data = [9u8; 40];
        // before the boundary: fine
        let mut r = reader(stored_then_bad(&data));
        let mut b = [0u8; 39];
        assert_eq!(r.read_full(&mut b).unwrap(), 39);
        // a read reaching it exactly, or past it: the error, then always
        assert!(r.read_full(&mut [0u8; 1]).is_err());
        assert!(r.read_full(&mut [0u8; 1]).is_err());
        let mut r = reader(stored_then_bad(&data));
        assert!(r.read_full(&mut [0u8; 40]).is_err());
        // a zero-length read meets nothing
        let mut r = reader(stored_then_bad(&data));
        assert_eq!(r.read_full(&mut [0u8; 39]).unwrap(), 39);
        assert_eq!(r.read_full(&mut []).unwrap(), 0);
        // nor after the error
        assert!(r.read_full(&mut [0u8; 1]).is_err());
        assert_eq!(r.read_full(&mut []).unwrap(), 0);
    }
}
