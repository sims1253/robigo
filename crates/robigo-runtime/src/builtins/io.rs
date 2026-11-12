//! Input/output: connections (GNU R's connections.c), `cat` (builtin.c
//! `do_cat`) and `readline` (scan.c).
//!
//! The connection table lives in `Interp::conns`. Connections 0, 1 and 2
//! are the terminals; `file()` and `textConnection()` connections take
//! the next free slot, as in R. Output written with `Rprintf` (all
//! printing) goes through `Interp::write_out`, which honours `sink()`.

use std::borrow::Cow;
use std::fs::File;
use std::io::{Read, Write};
use std::rc::Rc;

use super::{arg, internal};
use crate::coerce::{as_integer, as_logical};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::format::{
    PrintParams, encode_complex_dec, encode_integer, encode_logical, encode_real_dec, format_complex, format_real,
};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("cat", do_cat, 6).invisible(),
    internal("stdin", do_stdin, 0),
    internal("stdout", do_stdout, 0),
    internal("stderr", do_stderr, 0),
    internal("isatty", do_isatty, 1),
    internal("file", do_file, 6),
    internal("url", do_url, 6),
    internal("gzfile", do_gzfile, 4),
    internal("bzfile", do_gzfile, 4),
    internal("xzfile", do_gzfile, 4),
    internal("zstdfile", do_gzfile, 4),
    internal("memCompress", do_mem_compress, 2),
    internal("memDecompress", do_mem_decompress, 2),
    internal("textConnection", do_textconnection, 5),
    internal("textConnectionValue", do_textconvalue, 1),
    internal("open", do_open, 3).invisible(),
    internal("close", do_close, 2).invisible(),
    internal("isOpen", do_isopen, 2),
    internal("isIncomplete", do_isincomplete, 1),
    internal("isSeekable", do_isseekable, 1),
    internal("flush", do_flush, 1).invisible(),
    internal("readLines", do_readlines, 6),
    internal("writeLines", do_writelines, 4).invisible(),
    internal("readline", do_readln, 1),
    internal("sink", do_sink, 4).invisible(),
    internal("sink.number", do_sinknumber, 1),
    internal("summary.connection", do_sumconnection, 1),
    internal("getConnection", do_getconnection, 1),
    internal("getAllConnections", do_getallconnections, 0),
    internal("pushBack", do_pushback, 4).invisible(),
    internal("pushBackLength", do_pushbacklength, 1),
    internal("clearPushBack", do_clearpushback, 1).invisible(),
    internal("rawConnection", do_rawconnection, 3),
    internal("rawConnectionValue", do_rawconvalue, 1),
    internal("seek", do_seek, 4),
    internal("truncate", do_truncate, 1),
    internal("fifo", do_fifo, 4),
    internal("readBin", do_readbin, 6),
    internal("gzcon", do_gzcon, 4),
    internal("writeBin", do_writebin, 5).custom_vis(),
    internal("readChar", do_readchar, 3),
    internal("writeChar", do_writechar, 5).custom_vis(),
];

const NCONNECTIONS: usize = 128;
/// R: connections.c RBUFFCON_LEN_DEFAULT, the size of `con->buff`.
const RBUFFCON_LEN: usize = 4096;
const NSINKS: usize = 21;
/// `R_EOF`
pub(super) const EOF: i32 = -1;

// ----------------------------------------------------------------------
// The connection table

pub enum ConnKind {
    /// stdin (0), stdout (1) or stderr (2).
    Terminal,
    File {
        raw: bool,
        fp: Option<File>,
        rbuf: Vec<u8>,
        rpos: usize,
        wbuf: Vec<u8>,
        temp: bool,
        /// R: connections.c fileconn — whether the last operation was a
        /// write, to pick the position `seek()` reports.
        last_was_write: bool,
        /// R: fileconn rpos/wpos — the file's read and write positions,
        /// kept apart: switching between reading and writing seeks to the
        /// other one (an anonymous `file()` written then read starts
        /// reading at the beginning).
        frpos: u64,
        wpos: u64,
        /// A `fifo()` connection: opened with `fifo_open` (mkfifo when
        /// writing, O_NONBLOCK unless blocking) and never buffered.
        fifo: bool,
        /// R: `con->buff` is set — a file opened for reading in text mode
        /// (connections.c:set_buffer). `rbuf` is then R's connection
        /// buffer, filled [`RBUFFCON_LEN`] bytes at a time, and the file
        /// position counts what it holds (`seek()` subtracts it only when
        /// telling, as buff_seek does). Otherwise `rbuf` stands for
        /// stdio's buffer, which file positions never see.
        buffered: bool,
    },
    /// `textConnection(object, "r")`: the lines joined with newlines.
    TextIn { data: Vec<u8>, cur: usize },
    /// `textConnection(name, "w")`: lines go to a character vector bound
    /// to `namesym` in `env` (or kept internally for `NULL`).
    TextOut { namesym: Option<Sym>, env: Env, data: Vec<RStr>, lastline: Vec<u8> },
    /// `gzfile()`/`bzfile()`/`xzfile()`: read by decompressing the whole
    /// file when opened (uncompressed files are read as they are), written
    /// by compressing the buffered output when closed. `level` is the
    /// `compress` argument (negative for xz's extreme presets). An xz
    /// file whose decoder fails is read by `xz` as R reads it instead;
    /// `data` and `cur` are then a text connection's buffer (R's
    /// `con->buff`), empty in binary mode.
    Gz {
        codec: Codec,
        level: i32,
        data: Vec<u8>,
        cur: usize,
        out: Vec<u8>,
        writing: bool,
        append: bool,
        xz: Option<Box<XzLive>>,
    },
    /// `rawConnection()`: an in-memory raw vector (R: connections.c
    /// rawconn). `pos` is shared for reading and writing; `nbytes` may be
    /// smaller than `data.len()` (the allocation is over-sized).
    Raw { data: Vec<u8>, pos: usize, nbytes: usize },
    /// `url()` over http/https/ftp(s) (R: libcurl.c's url-libcurl):
    /// fetched into memory when opened, read from there. `headers` are
    /// the request's extra headers (`url(headers = )`).
    Url { data: Vec<u8>, cur: usize, headers: Vec<String> },
    /// `gzcon(con, ...)`: a gzip wrapper over another binary connection
    /// (connections.c's gzconn). Reading pulls the wrapped stream at open
    /// and decompresses it whole, as `gzfile()` does; writing deflates
    /// into the wrapped connection and finishes with the gzip trailer at
    /// close (gzcon_write/gzcon_close).
    Gzcon {
        /// The wrapped connection, out of the table (R: `Connections[icon]
        /// = new`); lent to a hidden slot while the gzcon does I/O on it.
        inner: Option<Box<Connection>>,
        level: u32,
        allow: bool,
        /// The decompressed stream, once open for reading.
        data: Vec<u8>,
        cur: usize,
        /// Writing: the uncompressed bytes, deflated with the crc and
        /// length trailer into the wrapped connection at close.
        out: Vec<u8>,
        writing: bool,
    },
    /// `socketConnection()` and `socketAccept()`.
    Sock(super::sock::SockConn),
    /// `serverSocket()`: open while it exists, it only accepts.
    ServSock { port: i32, fd: Option<std::os::fd::OwnedFd> },
    /// A connection C code made (`R_new_custom_connection`): its methods
    /// are the function pointers of the package's `struct Rconn` at `con`,
    /// which the native bridge owns and calls.
    Native { con: usize },
}

pub struct Connection {
    pub class: String,
    pub description: String,
    pub mode: String,
    pub text: bool,
    pub isopen: bool,
    pub incomplete: bool,
    /// R: `con->UTF8out`, requested by readLines() and scan() when they
    /// open the connection, kept by `set_iconv` only when it converts:
    /// their lines are then marked UTF-8.
    pub utf8out: bool,
    pub canread: bool,
    pub canwrite: bool,
    pub canseek: bool,
    pub blocking: bool,
    pub encname: String,
    /// The input re-encoding of a text connection opened for reading with
    /// an `encoding` (R: `con->inconv` and its buffers).
    inconv: Option<Box<InConv>>,
    pub status: i32,
    /// CR/LF mapping state of `Rconn_fgetc` (`-1000`: nothing saved), or
    /// [`SAVE_CONV`] while `inconv` is set, whose own state is
    /// [`InConv::save`], or [`SAVE_UNREAD`] while `unread` holds bytes of
    /// a connection without `inconv`, whose own state is [`Unread::save`].
    save: i32,
    /// Bytes read past where GNU R would have stopped reading, to be read
    /// again before the rest of the input (see [`unread`]).
    unread: Option<Box<Unread>>,
    /// Pushed-back lines, last pushed first out.
    pushback: Vec<Vec<u8>>,
    pos_pushback: usize,
    pub kind: ConnKind,
    /// The `conn_id` external pointer (none for a native connection: its
    /// own is a C object the bridge keeps, and the table must not keep it
    /// alive).
    pub ex_ptr: Value,
    /// GNU R's `con->id`: unique to the connection, never NULL.
    pub id: usize,
}

thread_local! {
    /// R: connections.c `current_id`
    static CURRENT_ID: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Connection {
    /// R: connections.c:init_con
    pub(super) fn new(class: &str, description: &str, mode: &str, kind: ConnKind) -> Connection {
        Connection::with_id(class, description, mode, kind, next_connection_id())
    }

    /// init_con with the id already taken from `current_id`.
    fn with_id(class: &str, description: &str, mode: &str, kind: ConnKind, id: usize) -> Connection {
        Connection {
            class: class.to_string(),
            description: description.to_string(),
            mode: mode.chars().take(4).collect(),
            text: true,
            isopen: false,
            incomplete: false,
            utf8out: false,
            canread: true,
            canwrite: true,
            canseek: false,
            blocking: false,
            encname: String::new(),
            inconv: None,
            status: NA_INTEGER,
            save: -1000,
            unread: None,
            pushback: Vec::new(),
            pos_pushback: 0,
            kind,
            ex_ptr: Value::Null,
            id,
        }
    }

    pub fn is_native(&self) -> bool {
        matches!(self.kind, ConnKind::Native { .. })
    }

    pub(super) fn reset_save(&mut self) {
        match (&mut self.inconv, &mut self.unread) {
            (Some(ic), _) => ic.save = -1000,
            (None, Some(u)) => u.save = -1000,
            (None, None) => self.save = -1000,
        }
    }

    /// Forget the unread bytes, as a seek, an open or a close does.
    fn drop_unread(&mut self) {
        if let Some(u) = self.unread.take() {
            if self.save == SAVE_UNREAD {
                self.save = u.save;
            }
        }
    }

    fn terminal(description: &str, mode: &str) -> Connection {
        let mut c = Connection::new("terminal", description, mode, ConnKind::Terminal);
        c.isopen = true;
        c.canread = mode == "r";
        c.canwrite = mode == "w";
        c
    }

    #[inline]
    pub fn is_contiguous_buffered(&self) -> bool {
        if !self.pushback.is_empty() || self.save != -1000 {
            return false;
        }
        matches!(
            &self.kind,
            ConnKind::File { fp: Some(_), fifo: false, .. }
                | ConnKind::TextIn { .. }
                | ConnKind::Url { .. }
                | ConnKind::Raw { .. }
                | ConnKind::Gz { xz: None, .. }
                | ConnKind::Gzcon { .. }
        )
    }

    /// Whether a short read means the end of the data: the kinds held in
    /// memory, which fill a read up to their end. R's readBin stops at a
    /// short read; a file or socket read can come back short before its
    /// end, which readBin retries.
    fn short_read_is_end(&self) -> bool {
        matches!(
            self.kind,
            ConnKind::Gz { .. }
                | ConnKind::Gzcon { .. }
                | ConnKind::TextIn { .. }
                | ConnKind::Raw { .. }
                | ConnKind::Url { .. }
        )
    }

    /// Whether reading needs the interpreter (`rconn_fgetc`): a native
    /// connection, an input conversion, or an xz decoder failure to warn
    /// about.
    fn reads_need_interp(&self) -> bool {
        self.is_native() || self.inconv.is_some() || matches!(self.kind, ConnKind::Gz { xz: Some(_), .. })
    }

    pub fn get_available_bytes(&mut self) -> Option<&[u8]> {
        if !self.pushback.is_empty() || self.save != -1000 {
            return None;
        }
        file_switch_to_refill(&mut self.kind);
        match &mut self.kind {
            ConnKind::File { fp: Some(f), rbuf, rpos, fifo, buffered, .. } => {
                if *fifo {
                    return None;
                }
                if *rpos >= rbuf.len() {
                    rbuf.resize(if *buffered { RBUFFCON_LEN } else { 65536 }, 0);
                    match f.read(rbuf) {
                        Ok(k) if k > 0 => {
                            rbuf.truncate(k);
                            *rpos = 0;
                        }
                        _ => {
                            rbuf.clear();
                            *rpos = 0;
                            return None;
                        }
                    }
                }
                Some(&rbuf[*rpos..])
            }
            ConnKind::TextIn { data, cur } | ConnKind::Url { data, cur, .. } => {
                if *cur >= data.len() {
                    None
                } else {
                    Some(&data[*cur..])
                }
            }
            ConnKind::Raw { data, pos, nbytes } => {
                if *pos >= *nbytes {
                    None
                } else {
                    Some(&data[*pos..*nbytes])
                }
            }
            ConnKind::Gz { data, cur, xz: None, .. } | ConnKind::Gzcon { data, cur, .. } => {
                if *cur >= data.len() {
                    None
                } else {
                    Some(&data[*cur..])
                }
            }
            _ => None,
        }
    }

    #[inline]
    pub fn consume_bytes(&mut self, count: usize) {
        match &mut self.kind {
            ConnKind::File { rpos, .. } => {
                *rpos += count;
            }
            ConnKind::TextIn { cur, .. } | ConnKind::Url { cur, .. } => {
                *cur += count;
            }
            ConnKind::Raw { pos, .. } => {
                *pos += count;
            }
            ConnKind::Gz { cur, .. } | ConnKind::Gzcon { cur, .. } => {
                *cur += count;
            }
            _ => {}
        }
    }

    pub fn scan_buffered(&mut self, sepchar: i32, strip: bool, discard_spaces: bool, buf: &mut Vec<u8>) -> i32 {
        let rspace = |b: u8| b == b' ' || b == b'\t' || b == b'\n' || b == b'\r';
        buf.clear();
        if sepchar == 0 {
            // Whitespace separated: skip leading ' ' and '\t'
            loop {
                let Some(slice) = self.get_available_bytes() else {
                    return EOF;
                };
                if let Some(p) = slice.iter().position(|&b| b != b' ' && b != b'\t') {
                    self.consume_bytes(p);
                    break;
                } else {
                    let len = slice.len();
                    self.consume_bytes(len);
                }
            }
            let Some(slice) = self.get_available_bytes() else {
                return EOF;
            };
            if slice[0] == b'\n' {
                self.consume_bytes(1);
                return b'\n' as i32;
            }
            if slice[0] == b'\r' {
                self.consume_bytes(1);
                if let Some(next) = self.get_available_bytes() {
                    if !next.is_empty() && next[0] == b'\n' {
                        self.consume_bytes(1);
                    }
                }
                return b'\n' as i32;
            }
            // Read non-whitespace token
            while let Some(slice) = self.get_available_bytes() {
                if let Some(pos) = slice.iter().position(|&b| rspace(b)) {
                    buf.extend_from_slice(&slice[..pos]);
                    self.consume_bytes(pos);
                    break;
                } else {
                    buf.extend_from_slice(slice);
                    let len = slice.len();
                    self.consume_bytes(len);
                }
            }
            // Skip trailing spaces/tabs on line
            loop {
                let Some(slice) = self.get_available_bytes() else {
                    return EOF;
                };
                if let Some(p) = slice.iter().position(|&b| b != b' ' && b != b'\t') {
                    self.consume_bytes(p);
                    break;
                } else {
                    let len = slice.len();
                    self.consume_bytes(len);
                }
            }
            let Some(slice) = self.get_available_bytes() else {
                return EOF;
            };
            if slice[0] == b'\n' {
                self.consume_bytes(1);
                return b'\n' as i32;
            }
            if slice[0] == b'\r' {
                self.consume_bytes(1);
                if let Some(next) = self.get_available_bytes() {
                    if !next.is_empty() && next[0] == b'\n' {
                        self.consume_bytes(1);
                    }
                }
                return b'\n' as i32;
            }
            1
        } else {
            let sep = sepchar as u8;
            let filled;
            loop {
                let Some(slice) = self.get_available_bytes() else {
                    filled = EOF;
                    break;
                };
                if let Some(pos) = slice.iter().position(|&b| b == sep || b == b'\n' || b == b'\r') {
                    let d = slice[pos];
                    buf.extend_from_slice(&slice[..pos]);
                    if d == b'\r' {
                        self.consume_bytes(pos + 1);
                        if let Some(next) = self.get_available_bytes() {
                            if !next.is_empty() && next[0] == b'\n' {
                                self.consume_bytes(1);
                            }
                        }
                        filled = b'\n' as i32;
                    } else {
                        self.consume_bytes(pos + 1);
                        filled = d as i32;
                    }
                    break;
                } else {
                    buf.extend_from_slice(slice);
                    let len = slice.len();
                    self.consume_bytes(len);
                }
            }
            if discard_spaces {
                buf.retain(|&b| b != b' ' && b != b'\t');
            } else if strip && !buf.is_empty() {
                let start = buf.iter().position(|&b| !rspace(b)).unwrap_or(buf.len());
                let end = buf.iter().rposition(|&b| !rspace(b)).map(|p| p + 1).unwrap_or(0);
                if start >= end {
                    buf.clear();
                } else {
                    buf.drain(..start);
                    buf.truncate(end - start);
                }
            }
            filled
        }
    }
}

const FILE_WBUF_SIZE: usize = 8192;

/// The codec of a compressed-file connection. R's `gzfile()` in read
/// mode and `file()` dispatch on the file's magic (connections.c:
/// comp_type_from_file) and open a bzfile connection for bzip2 data;
/// the connection's class then says how it was really opened.
// R: connections.c:newgzfile, newbzfile, newxzfile
#[derive(Clone, Copy, PartialEq)]
pub enum Codec {
    Gzip,
    Bzip2,
    /// xz, or with `alone` the legacy .lzma format (R's xzfile `type` 1,
    /// detected from the magic number only).
    Xz {
        alone: bool,
    },
}

impl Codec {
    /// The connection class R gives it.
    fn class(self) -> &'static str {
        match self {
            Codec::Gzip => "gzfile",
            Codec::Bzip2 => "bzfile",
            Codec::Xz { .. } => "xzfile",
        }
    }
}

/// liblzma's decoder memory limit in R's xz readers ("probably about
/// 80Mb is required, but 512Mb seems OK as a limit").
const XZ_MEMLIMIT: u64 = 536870912;

/// The `lzma_ret` code of a liblzma error, as R's messages print it.
fn lzma_ret(e: liblzma::stream::Error) -> i32 {
    use liblzma::stream::Error as E;
    match e {
        E::NoCheck => 2,
        E::UnsupportedCheck => 3,
        E::Mem => 5,
        E::MemLimit => 6,
        E::Format => 7,
        E::Options => 8,
        E::Data => 9,
        E::Program => 11,
    }
}

/// The decoder of an xzfile connection: the concatenated .xz streams, or
/// a .lzma file with `alone`.
// R: connections.c:xzfile_open
fn xz_decoder(alone: bool) -> Result<liblzma::stream::Stream, liblzma::stream::Error> {
    use liblzma::stream::{CONCATENATED, Stream};
    if alone { Stream::new_lzma_decoder(XZ_MEMLIMIT) } else { Stream::new_stream_decoder(XZ_MEMLIMIT, CONCATENATED) }
}

/// `BUFSIZ` on glibc: how much of the file each `fread` of
/// connections.c:xzfile_read asks for.
const XZ_BUFSIZ: usize = 8192;

/// R: connections.c:RBUFFCON_LEN_DEFAULT — what a text connection's buffer
/// fill (`buff_fill`) asks of `con->read`.
const RBUFFCON_LEN_DEFAULT: usize = 4096;

/// The reader of an xzfile connection whose decoder fails: GNU R's
/// `xzfile_read`, run as R runs it. R decodes as it reads and warns at
/// every read whose `lzma_code` calls end in an error; whether a read
/// that fills its request meets the error too depends on how far liblzma
/// gets within the call, so the reads must be R's own, with R's request
/// sizes. A healthy file (no failure) is decoded whole at open instead.
// R: connections.c:xzfileconn, xzfile_read
pub struct XzLive {
    strm: liblzma::stream::Stream,
    /// The compressed file; `fread` has read `raw[..fed]`.
    raw: Vec<u8>,
    fed: usize,
    /// Where the input not yet consumed starts (`stream.next_in`; the
    /// input is `raw[next_in..fed]`).
    next_in: usize,
    /// `xz->action == LZMA_FINISH`: `fread` met the end of the file.
    finish: bool,
    /// Warnings of reads made where no interpreter is at hand (an input
    /// conversion's fill, [`dummy_fgetc`]), for [`rconn_fgetc_conv`].
    queued: Vec<String>,
}

impl std::fmt::Debug for XzLive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XzLive").field("fed", &self.fed).field("next_in", &self.next_in).finish_non_exhaustive()
    }
}

impl XzLive {
    // R: connections.c:xzfile_open
    fn new(raw: Vec<u8>, alone: bool) -> Result<Self, liblzma::stream::Error> {
        Ok(Self { strm: xz_decoder(alone)?, raw, fed: 0, next_in: 0, finish: false, queued: Vec::new() })
    }

    /// `xzfile_read(buf, 1, buf.len(), con)`: how many bytes it gave, and
    /// the warning it gave.
    // R: connections.c:xzfile_read
    fn read(&mut self, buf: &mut [u8]) -> (usize, Option<String>) {
        use liblzma::stream::{Action, Error as E, Status};
        let mut given = 0;
        if buf.is_empty() {
            return (0, None);
        }
        loop {
            if self.next_in == self.fed && !self.finish {
                let k = XZ_BUFSIZ.min(self.raw.len() - self.fed);
                self.fed += k;
                // feof(): a regular file's fread comes back short only at
                // its end
                self.finish = k < XZ_BUFSIZ;
            }
            let action = if self.finish { Action::Finish } else { Action::Run };
            let (in0, out0) = (self.strm.total_in(), self.strm.total_out());
            let ret = self.strm.process(&self.raw[self.next_in..self.fed], &mut buf[given..], action);
            self.next_in += (self.strm.total_in() - in0) as usize;
            given += (self.strm.total_out() - out0) as usize;
            let msg = match ret {
                Ok(Status::Ok) if given == buf.len() => return (given, None),
                Ok(Status::Ok) => continue,
                Ok(Status::StreamEnd) => return (given, None),
                Ok(Status::GetCheck) => "lzma decoding result 4".to_string(),
                Ok(Status::MemNeeded) => "lzma decoding result 10".to_string(),
                Err(E::Mem | E::MemLimit) => "lzma decoder needed more memory".to_string(),
                Err(E::Format) => "lzma decoder format error".to_string(),
                Err(E::Data) => "lzma decoder corrupt data".to_string(),
                Err(e) => format!("lzma decoding result {}", lzma_ret(e)),
            };
            return (given, Some(msg));
        }
    }

    /// The rest of the decoded file, for [`con_read_all`] (which does not
    /// warn).
    fn read_rest(&mut self, out: &mut Vec<u8>) {
        let mut chunk = vec![0u8; 1 << 16];
        loop {
            let (k, warn) = self.read(&mut chunk);
            out.extend_from_slice(&chunk[..k]);
            if k < chunk.len() || warn.is_some() {
                return;
            }
        }
    }
}

/// Decode a whole xz (or .lzma) file as reading an xzfile connection to
/// its end does. `Ok(None)` is a decoder that fails somewhere, whose reads
/// [`XzLive`] makes; `Err` is a decoder that cannot be initialized.
// R: connections.c:xzfile_open, xzfile_read
fn xz_decode_file(raw: &[u8], alone: bool) -> Result<Option<Vec<u8>>, String> {
    use liblzma::stream::{Action, Status};
    let mut out = Vec::new();
    let mut strm = xz_decoder(alone).map_err(|e| format!("cannot initialize lzma decoder, error {}", lzma_ret(e)))?;
    loop {
        out.reserve(raw.len().max(1 << 16));
        let pos = strm.total_in() as usize;
        match strm.process_vec(&raw[pos..], &mut out, Action::Finish) {
            Ok(Status::StreamEnd) => return Ok(Some(out)),
            Ok(Status::Ok) => {}
            // LZMA_BUF_ERROR (a truncated stream: output space is never
            // what is missing), or an error
            Ok(Status::MemNeeded | Status::GetCheck) | Err(_) => return Ok(None),
        }
    }
}

/// Compress `data` into one .xz stream with LZMA2 at `preset` and a
/// CRC32 check, as xzfile connections and memCompress() write it.
// R: connections.c:xzfile_open (encoder), do_memCompress case 4
fn xz_encode(data: &[u8], preset: u32) -> Result<Vec<u8>, i32> {
    use liblzma::stream::{Action, Check, Filters, LzmaOptions, Status, Stream};
    let opts = LzmaOptions::new_preset(preset).map_err(lzma_ret)?;
    let mut filters = Filters::new();
    filters.lzma2(&opts);
    let mut strm = Stream::new_stream_encoder(&filters, Check::Crc32).map_err(lzma_ret)?;
    let mut out = Vec::with_capacity(data.len() + data.len() / 100 + 600);
    loop {
        out.reserve(1 << 16);
        let pos = strm.total_in() as usize;
        match strm.process_vec(&data[pos..], &mut out, Action::Finish) {
            Ok(Status::StreamEnd) => return Ok(out),
            Ok(Status::Ok) => {}
            Ok(Status::GetCheck) => return Err(4),
            Ok(Status::MemNeeded) => return Err(10),
            Err(e) => return Err(lzma_ret(e)),
        }
    }
}

/// R's xz preset for a `compress` level: its magnitude, extreme when
/// negative.
fn xz_preset(level: i32) -> u32 {
    let p = level.unsigned_abs();
    if level < 0 { p | liblzma::stream::PRESET_EXTREME } else { p }
}

fn flush_file_wbuf(f: &mut File, wbuf: &mut Vec<u8>) -> std::io::Result<()> {
    if !wbuf.is_empty() {
        f.write_all(wbuf)?;
        wbuf.clear();
    }
    Ok(())
}

fn file_write_buffered(f: &mut File, wbuf: &mut Vec<u8>, fifo: bool, bytes: &[u8]) -> std::io::Result<()> {
    if fifo {
        return f.write_all(bytes);
    }
    if bytes.is_empty() {
        return Ok(());
    }
    if !wbuf.is_empty() {
        if wbuf.len() + bytes.len() <= FILE_WBUF_SIZE {
            wbuf.extend_from_slice(bytes);
            if wbuf.len() == FILE_WBUF_SIZE {
                f.write_all(wbuf)?;
                wbuf.clear();
            }
            return Ok(());
        }
        f.write_all(wbuf)?;
        wbuf.clear();
    }
    if bytes.len() >= FILE_WBUF_SIZE {
        f.write_all(bytes)
    } else {
        if wbuf.capacity() < FILE_WBUF_SIZE {
            wbuf.reserve(FILE_WBUF_SIZE);
        }
        wbuf.extend_from_slice(bytes);
        Ok(())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let ConnKind::File { fp: Some(f), wbuf, .. } = &mut self.kind {
            let _ = flush_file_wbuf(f, wbuf);
            let _ = f.flush();
        }
    }
}

pub struct ConnState {
    pub table: Vec<Option<Box<Connection>>>,
    /// `R_OutputCon`: where `Rprintf` output goes.
    pub output_con: i32,
    /// `R_ErrorCon`: where `REprintf` output goes.
    pub error_con: i32,
    pub sink_number: usize,
    sink_cons: [i32; NSINKS],
    sink_close: [i32; NSINKS],
    sink_split: [bool; NSINKS],
}

impl Default for ConnState {
    /// R: connections.c:InitConnections
    fn default() -> Self {
        let mut table: Vec<Option<Box<Connection>>> = (0..NCONNECTIONS).map(|_| None).collect();
        table[0] = Some(Box::new(Connection::terminal("stdin", "r")));
        table[1] = Some(Box::new(Connection::terminal("stdout", "w")));
        table[2] = Some(Box::new(Connection::terminal("stderr", "w")));
        let mut sink_cons = [0; NSINKS];
        sink_cons[0] = 1;
        ConnState {
            table,
            output_con: 1,
            error_con: 2,
            sink_number: 0,
            sink_cons,
            sink_close: [0; NSINKS],
            sink_split: [false; NSINKS],
        }
    }
}

impl ConnState {
    /// R: connections.c:getConnection
    pub fn get(&mut self, n: i32) -> RResult<&mut Connection> {
        // (slots from NCONNECTIONS on are hidden ones, see `hide`)
        if n < 0 || n as usize >= self.table.len() || n == NA_INTEGER {
            return Err(Flow::error("invalid connection"));
        }
        match self.table[n as usize].as_deref_mut() {
            Some(c) => Ok(c),
            None => Err(Flow::error("invalid connection")),
        }
    }

    /// A slot outside R's connection table (from `NCONNECTIONS` on, so
    /// `getAllConnections()` does not list it and `next_connection` never
    /// hands it out): the connection a `gzcon` wraps.
    fn hide(&mut self, con: Box<Connection>) -> i32 {
        let k = match (NCONNECTIONS..self.table.len()).find(|&k| self.table[k].is_none()) {
            Some(k) => k,
            None => {
                self.table.push(None);
                self.table.len() - 1
            }
        };
        self.table[k] = Some(con);
        k as i32
    }

    fn next_connection(&self) -> RResult<usize> {
        (3..NCONNECTIONS)
            .find(|&i| self.table[i].is_none())
            .ok_or_else(|| Flow::error(format!("all {} connections are in use", NCONNECTIONS)))
    }

    /// R: connections.c:getActiveSink
    fn active_sink(&self, n: usize) -> i32 {
        if n >= self.sink_number {
            return 0;
        }
        if self.sink_split[self.sink_number - n] { self.sink_cons[self.sink_number - n - 1] } else { 0 }
    }
}

// ----------------------------------------------------------------------
// Opening, closing, reading and writing connections

/// The C library's message for an OS error, as R prints with
/// `strerror(errno)`.
pub fn os_error_message(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(code) => crate::sys::strerror(code),
        None => e.to_string(),
    }
}

/// `R_ExpandFileName`: tilde expansion.
pub fn expand_file_name(name: &str) -> String {
    if name == "~" || name.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}{}", home, &name[1..]);
        }
    }
    name.to_string()
}

// ----------------------------------------------------------------------
// The connection methods: one per function pointer of GNU R's
// `struct Rconn` (open, close, destroy, vfprintf, fgetc_internal, seek,
// truncate, fflush, read, write). Each matches every kind, so a new kind
// has to say what it does for each of them.

/// What a built-in kind's method gives for a native connection, whose
/// methods need the interpreter: the caller takes the bridge's path. The
/// match that tells is the one the method makes anyway, so the built-in
/// kinds pay nothing for native connections.
const NATIVE_GETC: i32 = i32::MIN;

/// `Connection::save` of a connection that re-encodes its input: the test
/// for a saved character, which every read makes anyway, sends the read to
/// `dummy_fgetc`, and keeps the connection off the contiguous-buffer paths.
const SAVE_CONV: i32 = -2000;

/// `Connection::save` of a connection without an input conversion that has
/// [`Connection::unread`] bytes: its reads take the cold path, as for
/// [`SAVE_CONV`].
const SAVE_UNREAD: i32 = -3000;

/// `Connection::save` after a CR whose next character is a read of a
/// failing xz decoder ([`xz_fgetc`]).
const SAVE_CR: i32 = -4000;

/// Bytes given back to a connection by [`unread`], with the connection's
/// `save` while they are read.
struct Unread {
    bytes: Vec<u8>,
    pos: usize,
    save: i32,
}

/// Where a reader of connection `n` began, for [`unread`]: `rpos` and
/// `Rconn_fgetc`'s CR/LF `save` of a buffered file connection whose next
/// bytes come straight from `rbuf` (no pushback, unread bytes or input
/// re-encoding before them).
pub(super) struct ReadMark(Option<(usize, i32)>);

pub(super) fn read_mark(i: &mut Interp, n: i32) -> RResult<ReadMark> {
    let con = i.conns.get(n)?;
    Ok(ReadMark(match &con.kind {
        ConnKind::File { buffered: true, rpos, .. }
            if con.pushback.is_empty() && con.unread.is_none() && con.inconv.is_none() =>
        {
            Some((*rpos, con.save))
        }
        _ => None,
    }))
}

/// Give back what the caller read from connection `n` since `mark` past
/// its first `kept` bytes, which GNU R would not have read:
/// `R_ParseConn`'s lexer reads a character at a time and stops where it
/// raised an error, so the rest stays in the connection's buffer
/// (`con->buff`), below the pushback (`pushBackLength()` does not count
/// it, `clearPushBack()` keeps it) and read before the rest of the input.
///
/// A buffered file connection that still holds those bytes in `rbuf` (as
/// read, before CR mapping) steps back to them, which leaves it as GNU R
/// is. Otherwise the bytes are kept as [`Unread`] bytes, as they were read
/// (CR mapped, and re-encoded if the connection re-encodes its input): a
/// seek drops them, and a tell does not count them (R: connections.c
/// buff_seek).
pub(super) fn unread(i: &mut Interp, n: i32, mark: &ReadMark, read: &[u8], kept: usize) -> RResult<()> {
    if kept >= read.len() {
        return Ok(());
    }
    let con = i.conns.get(n)?;
    if let (Some(start), ConnKind::File { rbuf, rpos, .. }) = (mark.0, &mut con.kind) {
        if let Some((p, save)) = replay_fgetc(rbuf, start, (*rpos, con.save), read, kept) {
            *rpos = p;
            con.save = save;
            return Ok(());
        }
    }
    let mut bytes = read[kept..].to_vec();
    match &mut con.unread {
        Some(u) => {
            bytes.extend_from_slice(&u.bytes[u.pos..]);
            u.bytes = bytes;
            u.pos = 0;
        }
        None => {
            let save = if con.save == SAVE_CONV { SAVE_CONV } else { std::mem::replace(&mut con.save, SAVE_UNREAD) };
            con.unread = Some(Box::new(Unread { bytes, pos: 0, save }));
        }
    }
    Ok(())
}

/// Replay `Rconn_fgetc` (CR and CRLF read as LF) over `rbuf` from the
/// position and `save` of `start`: when it reads `read` and ends at `end`,
/// `rbuf` still holds everything read since `start`, and the result is the
/// position and `save` after the first `kept` bytes.
// R: connections.c:Rconn_fgetc
fn replay_fgetc(rbuf: &[u8], start: (usize, i32), end: (usize, i32), read: &[u8], kept: usize) -> Option<(usize, i32)> {
    let (mut p, mut save) = start;
    let mut at = None;
    for (k, &b) in read.iter().enumerate() {
        if k == kept {
            at = Some((p, save));
        }
        let c = if save != -1000 {
            std::mem::replace(&mut save, -1000)
        } else {
            let c = i32::from(*rbuf.get(p)?);
            p += 1;
            if c == i32::from(b'\r') {
                let c2 = i32::from(*rbuf.get(p)?);
                p += 1;
                if c2 != i32::from(b'\n') {
                    save = if c2 != i32::from(b'\r') { c2 } else { i32::from(b'\n') };
                }
                i32::from(b'\n')
            } else {
                c
            }
        };
        if c != i32::from(b) {
            return None;
        }
    }
    if (p, save) == end { at } else { None }
}

/// The next unread byte of `con`, if it has one.
#[cold]
#[inline(never)]
fn unread_getc(con: &mut Connection) -> Option<i32> {
    let u = con.unread.as_mut()?;
    let c = u.bytes[u.pos] as i32;
    u.pos += 1;
    if u.pos >= u.bytes.len() {
        con.drop_unread();
    }
    Some(c)
}

/// Take up to `buf.len() - used` unread bytes of `con` into `buf[used..]`:
/// `used` plus how many it took.
fn unread_take(con: &mut Connection, buf: &mut [u8], used: usize) -> usize {
    let Some(u) = con.unread.as_mut() else { return used };
    let take = (buf.len() - used).min(u.bytes.len() - u.pos);
    buf[used..used + take].copy_from_slice(&u.bytes[u.pos..u.pos + take]);
    u.pos += take;
    if u.pos >= u.bytes.len() {
        con.drop_unread();
    }
    used + take
}
const NATIVE_READ: usize = usize::MAX;

/// The native bridge, which calls a native connection's methods.
fn native_bridge() -> RResult<&'static dyn crate::native::NativeBridge> {
    crate::native::bridge().ok_or_else(crate::native::no_native_connections)
}

/// Open connection `n` with its current mode (R's `con->open(con)`).
/// Failures are reported with a warning and `Ok(false)`.
pub(super) fn con_open(i: &mut Interp, n: i32) -> RResult<bool> {
    let con = i.conns.get(n)?;
    match con.kind {
        ConnKind::Native { .. } => native_bridge()?.conn_open(i, n),
        ConnKind::File { fifo: true, .. } => fifo_open(i, n),
        ConnKind::File { fifo: false, .. } => file_open(i, n),
        ConnKind::Gz { .. } => gz_open(i, n),
        ConnKind::Url { .. } => url_open(i, n),
        ConnKind::Gzcon { .. } => gzcon_open(i, n),
        ConnKind::Sock(_) => super::sock::sock_open(i, n),
        // open from their creation on (text_open, raw_open)
        ConnKind::Terminal
        | ConnKind::TextIn { .. }
        | ConnKind::TextOut { .. }
        | ConnKind::Raw { .. }
        | ConnKind::ServSock { .. } => {
            con.unread = None;
            con.save = -1000;
            Ok(true)
        }
    }
}

// R: connections.c:file_open
fn file_open(i: &mut Interp, n: i32) -> RResult<bool> {
    let con = i.conns.get(n)?;
    let mode = con.mode.clone();
    let description = con.description.clone();
    let (name, is_temp) = if description.is_empty() {
        (super::sysutils::tempfile_name(&i_tempdir()?, "Rf", ""), true)
    } else {
        (expand_file_name(&description), false)
    };
    let mut oo = std::fs::OpenOptions::new();
    let plus = mode.len() >= 2 && mode.as_bytes()[1] == b'+';
    match mode.as_bytes().first() {
        Some(b'w') => {
            oo.write(true).create(true).truncate(true);
            if plus {
                oo.read(true);
            }
        }
        Some(b'a') => {
            oo.append(true).create(true);
            if plus {
                oo.read(true);
            }
        }
        _ => {
            oo.read(true);
            if plus {
                oo.write(true);
            }
        }
    }
    let f = match oo.open(&name) {
        Ok(f) => f,
        Err(e) => {
            let msg = format!("cannot open file '{}': {}", name, os_error_message(&e));
            i.warning(msg)?;
            return Ok(false);
        }
    };
    if f.metadata().map(|m| m.is_dir()).unwrap_or(false) {
        i.warning(format!("cannot open file '{}': it is a directory", name))?;
        return Ok(false);
    }
    if is_temp {
        let _ = std::fs::remove_file(&name);
    }
    let con = i.conns.get(n)?;
    if let ConnKind::File { fp, rbuf, rpos, wbuf, temp, .. } = &mut con.kind {
        *fp = Some(f);
        rbuf.clear();
        *rpos = 0;
        wbuf.clear();
        *temp = is_temp;
    }
    con.isopen = true;
    let m = con.mode.as_bytes();
    con.canwrite = m.first() == Some(&b'w') || m.first() == Some(&b'a');
    con.canread = !con.canwrite;
    if m.len() >= 2 && m[1] == b'+' {
        con.canread = true;
        con.canwrite = true;
    }
    con.text = !(m.len() >= 2 && m[m.len() - 1] == b'b');
    con.unread = None;
    con.save = -1000;
    // R: connections.c:file_open → set_buffer
    let with_buff = con.canread && con.text;
    if let ConnKind::File { buffered, .. } = &mut con.kind {
        *buffered = with_buff;
    }
    init_file_positions(con);
    set_iconv(i, n)?;
    Ok(true)
}

/// Whether glibc's iconv can convert `from` to `to` ("" is the locale
/// charset, as R's `Riconv_open` on Unix relies on).
fn iconv_converts(to: &str, from: &str) -> bool {
    crate::sys::iconv_supported(to, from)
}

/// Whether opening the connection gave it an input buffer (R: `con->buff`):
/// `set_buffer` buffers a readable text connection, and only
/// connections.c:file_open on a regular file (`shouldBuffer`) and the
/// compressed-file openers call it. `set_iconv` needs a readable text
/// connection too, so the kind decides.
fn buffers_input(kind: &ConnKind) -> bool {
    match kind {
        // fifo_open calls no set_buffer, whatever the file is
        ConnKind::File { fp: Some(f), fifo: false, .. } => f.metadata().is_ok_and(|m| m.is_file()),
        ConnKind::Gz { .. } => true,
        _ => false,
    }
}

/// `set_iconv` (R: connections.c): a text connection with a non-native
/// `encoding` gets an input conversion when open for reading, which
/// `dummy_fgetc` applies: to UTF-8 when readLines() or scan() asked for it
/// outside a UTF-8 locale, else to the session charset. The connection is
/// destroyed with an error when the conversion does not exist. robigo
/// writes text as is, but checks the output conversion as R does.
fn set_iconv(i: &mut Interp, n: i32) -> RResult<()> {
    let (text, encname, canread, canwrite, utf8out) = {
        let con = i.conns.get(n)?;
        con.inconv = None;
        if con.save == SAVE_CONV {
            con.save = -1000;
        }
        (con.text, con.encname.clone(), con.canread, con.canwrite, con.utf8out)
    };
    if !text || encname.is_empty() || encname == "native.enc" {
        i.conns.get(n)?.utf8out = false;
        return Ok(());
    }
    if canread {
        // UTF8out is set in readLines() and scan()
        let use_utf8 = utf8out && !crate::sys::utf8locale();
        let to = if use_utf8 { "UTF-8" } else { "" };
        let enc = if encname == "UTF-8-BOM" { "UTF-8" } else { encname.as_str() };
        let Some(cd) = crate::sys::Iconv::open(to, enc) else {
            con_free(i, n)?;
            return Err(Flow::error(format!("unsupported conversion from '{}' to '{}'", encname, to)));
        };
        let mut ic = InConv {
            cd,
            iconvbuff: [0; 25],
            inavail: 0,
            oconvbuff: [0; 50],
            next: 0,
            navail: 0,
            eof_signalled: false,
            invalid: false,
            buffered: buffers_input(&i.conns.get(n)?.kind),
            save: -1000,
        };
        // initialize state, and prepare any initial bytes
        let mut ob: &mut [u8] = &mut ic.oconvbuff;
        ic.cd.reset(&mut ob);
        ic.navail = (50 - ob.len()) as i16;
        // glibc's iconv does not handle BOMs; discard them in dummy_fgetc
        if encname == "UCS-2LE" || encname == "UTF-16LE" {
            ic.inavail = -2;
        }
        if encname == "UTF-8-BOM" {
            ic.inavail = -3;
        }
        let con = i.conns.get(n)?;
        con.inconv = Some(Box::new(ic));
        con.save = SAVE_CONV;
    }
    if canwrite && !iconv_converts(encname.as_str(), "") {
        con_free(i, n)?;
        return Err(Flow::error(format!("unsupported conversion from '{}' to '{}'", encname, "")));
    }
    Ok(())
}

/// `con->inconv` with the buffers `dummy_fgetc` converts through: up to
/// 25 input bytes, up to 50 converted ones.
pub(super) struct InConv {
    cd: crate::sys::Iconv,
    iconvbuff: [u8; 25],
    /// Bytes in `iconvbuff`; negative while a BOM is still to be checked.
    inavail: i16,
    oconvbuff: [u8; 50],
    next: usize,
    navail: i16,
    eof_signalled: bool,
    /// Invalid input was met: `rconn_fgetc` warns, as `dummy_fgetc` does.
    invalid: bool,
    /// The connection buffers its input (R: `con->buff`, set by
    /// connections.c:set_buffer when the connection was opened).
    buffered: bool,
    /// `Rconn_fgetc`'s CR/LF state (`con->save`) for this connection.
    save: i32,
}

/// `dummy_fgetc` (R: connections.c) for a connection with an input
/// conversion: the next byte of the converted text.
#[cold]
#[inline(never)]
fn dummy_fgetc(con: &mut Connection) -> i32 {
    let Some(mut ic) = con.inconv.take() else {
        return con_fgetc_internal(con);
    };
    let c = dummy_fgetc_conv(con, &mut ic);
    con.inconv = Some(ic);
    c
}

fn dummy_fgetc_conv(con: &mut Connection, ic: &mut InConv) -> i32 {
    // R reads up to a line end only when the input is not buffered
    // anyway (PR17634).
    let unbuffered = !ic.buffered;
    while ic.navail <= 0 {
        let (mut check_bom, mut check_bom8) = (false, false);
        if ic.eof_signalled {
            return EOF;
        }
        if ic.inavail < 0 {
            match ic.inavail {
                -2 => {
                    ic.inavail = 0;
                    check_bom = true;
                }
                -3 => {
                    ic.inavail = 0;
                    check_bom8 = true;
                }
                -21 => {
                    ic.inavail = 1;
                    check_bom = true;
                }
                -31 => {
                    ic.inavail = 1;
                    check_bom8 = true;
                }
                -32 => {
                    ic.inavail = 2;
                    check_bom8 = true;
                }
                _ => {}
            }
        }
        let mut inew = 0;
        while (ic.inavail as usize) < ic.iconvbuff.len() {
            let mut c = con_fgetc_internal(con);
            if c == NATIVE_GETC {
                c = xz_fgetc_queued(con);
            }
            if c == EOF {
                // not EOF_signalled: a non-blocking read may succeed later
                break;
            }
            ic.iconvbuff[ic.inavail as usize] = c as u8;
            ic.inavail += 1;
            inew += 1;
            if unbuffered && (c == '\n' as i32 || c == '\r' as i32) {
                break;
            }
        }
        if check_bom || check_bom8 {
            // a partial BOM: remember how much of it is still to check
            if ic.inavail == 0 {
                ic.inavail = if check_bom { -2 } else { -3 };
                return EOF;
            }
            if ic.inavail == 1 {
                if check_bom && ic.iconvbuff[0] == 0xff {
                    ic.inavail = -21;
                    return EOF;
                }
                if check_bom8 && ic.iconvbuff[0] == 0xef {
                    ic.inavail = -31;
                    return EOF;
                }
            }
            if ic.inavail == 2 && check_bom8 && ic.iconvbuff[1] == 0xbb {
                ic.inavail = -32;
                return EOF;
            }
        }
        if inew == 0 {
            return EOF;
        }
        let drop = if check_bom && ic.inavail >= 2 && ic.iconvbuff[..2] == [0xff, 0xfe] {
            2
        } else if check_bom8 && ic.inavail >= 3 && ic.iconvbuff[..3] == *b"\xef\xbb\xbf" {
            3
        } else {
            0
        };
        if drop > 0 {
            ic.iconvbuff.copy_within(drop..ic.inavail as usize, 0);
            ic.inavail -= drop as i16;
        }
        let inbuf = ic.iconvbuff;
        let mut ib: &[u8] = &inbuf[..ic.inavail as usize];
        let mut ob: &mut [u8] = &mut ic.oconvbuff;
        let res = ic.cd.step(&mut ib, &mut ob);
        let (inb, onb) = (ib.len(), ob.len());
        let consumed = ic.inavail as usize - inb;
        ic.inavail = inb as i16;
        ic.next = 0;
        ic.navail = (50 - onb) as i16;
        match res {
            // incomplete input char or no space in output buffer
            Err(e) if e == libc::EINVAL || e == libc::E2BIG => {
                ic.iconvbuff.copy_within(consumed..consumed + inb, 0);
            }
            Err(_) => {
                let mut none: &mut [u8] = &mut [];
                ic.cd.reset(&mut none);
                ic.invalid = true;
                ic.inavail = 0;
                // read no more bytes, possibly those after the invalid ones
                ic.eof_signalled = true;
            }
            Ok(()) => {}
        }
    }
    ic.navail -= 1;
    ic.next += 1;
    ic.oconvbuff[ic.next - 1] as i32
}

/// What `R_ParseConn` reads: the whole connection, through `Rconn_fgetc`
/// when the connection re-encodes its input or its xz decoder failed
/// (R: gram.c:con_getc).
pub(super) fn con_read_all_parse(i: &mut Interp, n: i32) -> RResult<Vec<u8>> {
    let con = i.conns.get(n)?;
    if con.inconv.is_none() && !matches!(con.kind, ConnKind::Gz { xz: Some(_), .. }) {
        return con_read_all(i, n);
    }
    let mut out = Vec::new();
    loop {
        let c = rconn_fgetc(i, n)?;
        if c == EOF {
            return Ok(out);
        }
        out.push(c as u8);
    }
}

/// `dummy_fgetc`'s warning, which needs the interpreter.
#[cold]
#[inline(never)]
fn warn_invalid_input(i: &mut Interp, n: i32) -> RResult<()> {
    let con = i.conns.get(n)?;
    if let Some(ic) = con.inconv.as_mut().filter(|ic| ic.invalid) {
        ic.invalid = false;
        let desc = con.description.clone();
        i.warning(format!("invalid input found on input connection '{}'", desc))?;
    }
    Ok(())
}

// R: connections.c fifo_open's <fcntl.h> flags.
const O_RDONLY: i32 = 0o0;
const O_WRONLY: i32 = 0o1;
const O_RDWR: i32 = 0o2;
const O_APPEND: i32 = 0o2000;
const O_NONBLOCK: i32 = 0o4000;
const ENXIO: i32 = 6;

/// `fifo()`'s open: create the fifo when writing, then open it with
/// `O_NONBLOCK` unless the connection is blocking.
// R: connections.c fifo_open
fn fifo_open(i: &mut Interp, n: i32) -> RResult<bool> {
    let (description, mode, blocking) = {
        let con = i.conns.get(n)?;
        (con.description.clone(), con.mode.clone(), con.blocking)
    };
    let (name, is_temp) = if description.is_empty() {
        (super::sysutils::tempfile_name(&i_tempdir()?, "Rf", ""), true)
    } else {
        (expand_file_name(&description), false)
    };
    let m = mode.as_bytes();
    let canwrite = m.first() == Some(&b'w') || m.first() == Some(&b'a');
    let mut canread = !canwrite;
    if m.len() >= 2 && m[1] == b'+' {
        canread = true;
    }
    // If we are to write, create the fifo if needed.
    if canwrite {
        match std::fs::metadata(&name) {
            Err(_) => {
                let res = c_open_mkfifo(&name);
                if res != 0 {
                    let reason = os_error_message(&std::io::Error::from_raw_os_error(res));
                    i.warning(format!("cannot create fifo '{}', reason '{}'", name, reason))?;
                    return Ok(false);
                }
            }
            Ok(md) => {
                use std::os::unix::fs::FileTypeExt;
                if !md.file_type().is_fifo() {
                    i.warning(format!("'{}' exists but is not a fifo", name))?;
                    return Ok(false);
                }
            }
        }
    }
    let mut flags = if canread && canwrite {
        O_RDWR
    } else if canread {
        O_RDONLY
    } else {
        O_WRONLY
    };
    if !blocking {
        flags |= O_NONBLOCK;
    }
    if m.first() == Some(&b'a') {
        flags |= O_APPEND;
    }
    if name.as_bytes().contains(&0) {
        return Ok(false);
    }
    let f = match crate::sys::open(&name, flags) {
        Ok(f) => f,
        Err(err) => {
            if err == ENXIO {
                i.warning(format!("fifo '{}' is not ready", name))?;
            } else {
                i.warning(format!("cannot open fifo '{}'", name))?;
            }
            return Ok(false);
        }
    };
    if is_temp {
        let _ = std::fs::remove_file(&name);
    }
    let con = i.conns.get(n)?;
    if let ConnKind::File { fp, rbuf, rpos, wbuf, temp, .. } = &mut con.kind {
        *fp = Some(f);
        rbuf.clear();
        *rpos = 0;
        wbuf.clear();
        *temp = is_temp;
    }
    con.isopen = true;
    con.canwrite = canwrite;
    con.canread = canread;
    con.text = !(m.len() >= 2 && m[m.len() - 1] == b'b');
    con.unread = None;
    con.save = -1000;
    init_file_positions(con);
    set_iconv(i, n)?;
    Ok(true)
}

/// Open a `url()` connection (R: libcurl.c:Curl_open). The resource is
/// fetched whole here (`internet::url_fetch`) and read from memory.
fn url_open(i: &mut Interp, n: i32) -> RResult<bool> {
    let (url, mode, headers) = {
        let con = i.conns.get(n)?;
        let ConnKind::Url { headers, .. } = &con.kind else { unreachable!() };
        (con.description.clone(), con.mode.clone(), headers.clone())
    };
    // R: write modes print this (REprintf, no newline) and fail the open
    if !mode.starts_with('r') {
        i.write_err("can only open URLs for reading");
        return Ok(false);
    }
    let Some(data) = super::internet::url_fetch(i, &url, &headers)? else {
        return Err(Flow::error(format!("cannot open the connection to '{url}'")));
    };
    let con = i.conns.get(n)?;
    if let ConnKind::Url { data: d, cur, .. } = &mut con.kind {
        *d = data;
        *cur = 0;
    }
    con.isopen = true;
    con.canwrite = mode.starts_with('w') || mode.starts_with('a');
    con.canread = !con.canwrite;
    // R: text unless the mode ends in "b"
    con.text = !(mode.len() >= 2 && mode.ends_with('b'));
    con.unread = None;
    con.save = -1000;
    set_iconv(i, n)?;
    Ok(true)
}

/// `mkfifo(name, 00644)`, returning errno on failure (0 on success).
fn c_open_mkfifo(name: &str) -> i32 {
    crate::sys::mkfifo(name)
}

/// Open a `gzfile`/`bzfile`/`xzfile` connection.
// R: connections.c:gzfile_open, bzfile_open, xzfile_open
fn gz_open(i: &mut Interp, n: i32) -> RResult<bool> {
    let con = i.conns.get(n)?;
    let name = expand_file_name(&con.description);
    let m = con.mode.clone();
    let writing = m.starts_with('w') || m.starts_with('a');
    let mut data = Vec::new();
    let mut xz_live = None;
    if !writing {
        let raw = match std::fs::read(&name) {
            Ok(d) => d,
            Err(e) => {
                let bzed = matches!(con.kind, ConnKind::Gz { codec: Codec::Bzip2, .. });
                i.warning(format!(
                    "cannot open {} file '{}', probable reason '{}'",
                    if bzed { "bzip2-ed" } else { "compressed" },
                    name,
                    os_error_message(&e)
                ))?;
                return Ok(false);
            }
        };
        let codec = match con.kind {
            ConnKind::Gz { codec, .. } => codec,
            _ => Codec::Gzip,
        };
        match codec {
            Codec::Xz { alone } => {
                // R: connections.c:xzfile_open/xzfile_read — a decoder
                // that fails warns at the reads that meet the failure:
                // those read as R does
                match xz_decode_file(&raw, alone) {
                    Ok(Some(d)) => data = d,
                    Ok(None) => match XzLive::new(raw, alone) {
                        Ok(x) => xz_live = Some(Box::new(x)),
                        Err(e) => i.warning(format!("cannot initialize lzma decoder, error {}", lzma_ret(e)))?,
                    },
                    Err(w) => i.warning(w)?,
                }
            }
            Codec::Bzip2 => {
                // R: connections.c:bzfile_open — BZ2_bzReadOpen never
                // reads, so any file opens; data errors surface at read
                // (bzfile_read): only BZ_DATA_ERROR_MAGIC warns ("appears
                // not to be compressed"; a bad header in a later stream
                // after a decoded one warns "has trailing content" from
                // the re-open). Everything else — EOF mid-header (a bare
                // "BZh9") or mid-stream, a block-magic mismatch — is
                // silent and keeps the bytes read so far. The bzip2 crate
                // calls libbz2, so its error kinds are bzlib's codes; the
                // whole-file model decodes at open, so warnings fire here.
                // A valid empty stream (and an empty file) decodes to
                // nothing and opens.
                let mut err: Option<bzip2::Error> = None;
                let mut dec = bzip2::read::MultiBzDecoder::new(&raw[..]);
                let mut buf = [0u8; 8192];
                loop {
                    match dec.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => data.extend_from_slice(&buf[..n]),
                        Err(e) => {
                            err = e.get_ref().and_then(|x| x.downcast_ref::<bzip2::Error>().cloned());
                            break;
                        }
                    }
                }
                if matches!(err, Some(bzip2::Error::DataMagic)) && !raw.is_empty() {
                    if data.is_empty() {
                        i.warning(format!("file '{name}' appears not to be compressed by bzip2"))?;
                    } else {
                        i.warning(format!(
                            "file '{name}' has trailing content that appears not to be compressed by bzip2"
                        ))?;
                    }
                }
            }
            Codec::Gzip => {
                if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
                    // The flat cache (rdbflat.rs) may already hold this file's
                    // inflate — a lazy-load database's .rdx index is read with a
                    // gzfile at every startup; the builder pre-inflates it next
                    // to its .rdb's cache (single (0, 0) entry, same size/mtime
                    // guard). Falls through to the decoder when there is none.
                    let cached = crate::rdbflat::with_entry(&name, 0, 0, |d| d.to_vec());
                    if let Some(d) = cached {
                        data = d;
                    } else if flate2::read::MultiGzDecoder::new(&raw[..]).read_to_end(&mut data).is_err() {
                        return Err(Flow::error(format!("robigo: '{}' is not a valid gzip file", name)));
                    }
                } else {
                    data = raw;
                }
            }
        }
    } else if let Err(e) = std::fs::OpenOptions::new().create(true).append(true).open(&name) {
        let bzed = matches!(con.kind, ConnKind::Gz { codec: Codec::Bzip2, .. });
        i.warning(format!(
            "cannot open {} file '{}', probable reason '{}'",
            if bzed { "bzip2-ed" } else { "compressed" },
            name,
            os_error_message(&e)
        ))?;
        return Ok(false);
    } else if writing && matches!(&con.kind, ConnKind::Gz { codec: Codec::Bzip2, level: 0, .. }) {
        // R: connections.c:bzfile_open — BZ2_bzWriteOpen rejects
        // blockSize100k = 0 with BZ_CONFIG_ERROR; R warns and the open
        // fails (compress = 0 passes do_gzfile's 0..9 validation).
        i.warning(format!("initializing bzip2 compression for file '{name}' failed"))?;
        return Ok(false);
    }
    let con = i.conns.get(n)?;
    // R: connections.c:gzfile_open, bzfile_open, xzfile_open — text
    // unless the mode has a "b", and a text connection's encoding converts
    let text = !m.contains('b');
    if let ConnKind::Gz { data: d, cur, out, writing: w, append, xz, .. } = &mut con.kind {
        *xz = xz_live;
        *d = data;
        *cur = 0;
        out.clear();
        *w = writing;
        *append = m.starts_with('a');
    }
    con.isopen = true;
    con.canread = !writing;
    con.canwrite = writing;
    con.text = text;
    con.unread = None;
    con.save = -1000;
    set_iconv(i, n)?;
    Ok(true)
}

/// The description of connection `n`.
pub fn connection_description(i: &mut Interp, n: i32) -> RResult<String> {
    Ok(i.conns.get(n)?.description.clone())
}

/// `Rconn_fgetc` on connection `n` (`-1` at end of input).
/// The fields of connection `n` that GNU R's `struct Rconn` shows C code:
/// class, description, mode and (text, isopen, incomplete, canread,
/// canwrite, canseek, blocking).
pub fn conn_info(i: &mut Interp, n: i32) -> RResult<(String, String, String, [bool; 7])> {
    let c = i.conns.get(n)?;
    Ok((
        c.class.clone(),
        c.description.clone(),
        c.mode.clone(),
        [c.text, c.isopen, c.incomplete, c.canread, c.canwrite, c.canseek, c.blocking],
    ))
}

/// `con->open(con)` for C code: whether it opened.
pub fn conn_open(i: &mut Interp, n: i32) -> RResult<bool> {
    con_open(i, n)
}

/// `con->close(con)` for C code.
pub fn conn_close(i: &mut Interp, n: i32) -> RResult<()> {
    con_close_raw(i, n)
}

pub fn conn_getc(i: &mut Interp, n: i32) -> RResult<i32> {
    rconn_fgetc(i, n)
}

/// Does connection `n` hold pushed-back lines (R's `con->nPushBack > 0`)?
pub fn conn_has_pushback(i: &mut Interp, n: i32) -> RResult<bool> {
    Ok(!i.conns.get(n)?.pushback.is_empty())
}

/// Push a line back onto connection `n` (R's `con_pushback`).
pub fn conn_pushback(i: &mut Interp, n: i32, line: &[u8]) -> RResult<()> {
    let con = i.conns.get(n)?;
    if !line.is_empty() {
        con.pushback.push(line.to_vec());
        con.pos_pushback = 0;
    }
    Ok(())
}

/// State of connection `n`: (is open, can read, can write, text mode).
pub fn conn_state(i: &mut Interp, n: i32) -> RResult<(bool, bool, bool, bool)> {
    let c = i.conns.get(n)?;
    Ok((c.isopen, c.canread, c.canwrite, c.text))
}

/// Ask for a connection's input to be converted to UTF-8 (R:
/// `con->UTF8out = TRUE`) before opening it.
pub fn request_utf8out(i: &mut Interp, n: i32) -> RResult<()> {
    i.conns.get(n)?.utf8out = true;
    Ok(())
}

/// `strcpy(con->mode, mode); con->open(con)`: open connection `n` in
/// `mode`, which it keeps.
pub fn open_as(i: &mut Interp, n: i32, mode: &str) -> RResult<bool> {
    i.conns.get(n)?.mode = mode.to_string();
    con_open(i, n)
}

/// Open connection `n` in `mode` without changing its recorded mode, as
/// serialize.c does for connections it was given closed.
pub fn open_in_mode(i: &mut Interp, n: i32, mode: &str) -> RResult<bool> {
    let saved = std::mem::replace(&mut i.conns.get(n)?.mode, mode.to_string());
    let r = con_open(i, n);
    // an open that fails can destroy the connection (set_iconv), and its
    // error is the one to report
    if let Ok(con) = i.conns.get(n) {
        con.mode = saved;
    }
    r
}

/// `con->close(con)`.
pub fn close_conn(i: &mut Interp, n: i32) -> RResult<()> {
    con_close_raw(i, n)
}

/// The connection number of a connection object.
pub fn connection_number(v: &Value) -> RResult<i32> {
    con_arg(v)
}

/// Read the rest of a connection opened for binary reading (R's
/// `con->read` until end of input). Its callers stand for C code that
/// reads only what it needs (unserialize, load), so a failing xz decoder
/// does not warn here: such a reader meets the failure only on incomplete
/// data, which it reports.
pub fn con_read_all(i: &mut Interp, n: i32) -> RResult<Vec<u8>> {
    let con = i.conns.get(n)?;
    let mut out: Vec<u8> = Vec::new();
    while con.pos_pushback < con.pushback.len() {
        // Pushed-back lines come first.
        let line = con.pushback.pop().unwrap_or_default();
        out.extend_from_slice(&line);
    }
    if let Some(u) = con.unread.as_ref() {
        out.extend_from_slice(&u.bytes[u.pos..]);
        con.drop_unread();
    }
    file_switch_to_read(&mut con.kind);
    match &mut con.kind {
        ConnKind::Gz { data, cur, xz, .. } => {
            out.extend_from_slice(&data[*cur..]);
            *cur = data.len();
            if let Some(x) = xz {
                x.read_rest(&mut out);
            }
        }
        ConnKind::Gzcon { data, cur, .. } | ConnKind::TextIn { data, cur } | ConnKind::Url { data, cur, .. } => {
            out.extend_from_slice(&data[*cur..]);
            *cur = data.len();
        }
        ConnKind::Raw { data, pos, nbytes } => {
            out.extend_from_slice(&data[*pos..*nbytes]);
            *pos = *nbytes;
        }
        ConnKind::File { fp: Some(f), rbuf, rpos, last_was_write, .. } => {
            *last_was_write = false;
            out.extend_from_slice(&rbuf[*rpos..]);
            *rpos = rbuf.len();
            let _ = f.read_to_end(&mut out);
        }
        ConnKind::Terminal => loop {
            let c = crate::console::getchar();
            if c == EOF {
                break;
            }
            out.push(c as u8);
        },
        ConnKind::Sock(_) | ConnKind::Native { .. } => {
            let mut chunk = [0u8; 4096];
            loop {
                let k = con_read_bytes(i, n, &mut chunk)?;
                out.extend_from_slice(&chunk[..k]);
                if k < chunk.len() {
                    break;
                }
            }
        }
        ConnKind::File { fp: None, .. } | ConnKind::TextOut { .. } | ConnKind::ServSock { .. } => {
            return Err(Flow::error("cannot read from this connection"));
        }
    }
    Ok(out)
}

/// Write bytes to a connection (R's `con->write`).
pub fn con_write_bytes(i: &mut Interp, n: i32, bytes: &[u8]) -> RResult<()> {
    con_write_bytes_or(i, n, bytes, conn_write)
}

/// [`con_write_bytes`], with `text` for the connections that take text
/// (the terminals and text connections, as Rconn_printf from C through
/// con->vfprintf, as R's write.table does to stdout()): only that path can
/// run R code (a text connection assigns its lines; `sink()` diverts the
/// terminals), which the C API's callback wraps. A file, gzfile, raw or
/// socket connection takes the bytes as they are.
#[inline(always)]
pub fn con_write_bytes_or(
    i: &mut Interp,
    n: i32,
    bytes: &[u8],
    text: impl FnOnce(&mut Interp, i32, &str) -> RResult<()>,
) -> RResult<()> {
    let byte_kind = matches!(
        i.conns.get(n)?.kind,
        ConnKind::Gz { writing: true, .. }
            | ConnKind::Gzcon { writing: true, .. }
            | ConnKind::Raw { .. }
            | ConnKind::File { fp: Some(_), .. }
            | ConnKind::Sock(_)
            | ConnKind::Native { .. }
    );
    if n <= 2 || !byte_kind {
        return text(i, n, &String::from_utf8_lossy(bytes));
    }
    if con_write(i.conns.get(n)?, bytes)? {
        return Ok(());
    }
    native_bridge()?.conn_write(i, n, bytes)?;
    Ok(())
}

/// `con->write(bytes, 1, bytes.len(), con)`; false for a native
/// connection, whose method the caller calls.
#[inline(always)]
fn con_write(con: &mut Connection, bytes: &[u8]) -> RResult<bool> {
    file_switch_to_write(&mut con.kind);
    match &mut con.kind {
        ConnKind::Gz { out, writing: true, .. } => {
            out.extend_from_slice(bytes);
            Ok(true)
        }
        // R: connections.c:gzcon_write — buffered; the deflate stream, crc
        // and length are computed over the whole buffer at close.
        ConnKind::Gzcon { out, writing: true, .. } => {
            out.extend_from_slice(bytes);
            Ok(true)
        }
        ConnKind::Raw { data, pos, nbytes } => {
            // R: connections.c raw_write + raw_resize's growth policy
            let freespace = data.len().saturating_sub(*pos);
            if bytes.len() >= freespace {
                let needed = bytes.len() + *pos;
                let mut nalloc = 64usize;
                if needed > 8192 {
                    nalloc = needed + needed / 5; // 20% over-allocation
                } else {
                    while nalloc < needed {
                        nalloc *= 2;
                    }
                }
                data.resize(nalloc, 0);
            }
            data[*pos..*pos + bytes.len()].copy_from_slice(bytes);
            *pos += bytes.len();
            if *nbytes < *pos {
                *nbytes = *pos;
            }
            Ok(true)
        }
        ConnKind::File { fp: Some(f), wbuf, fifo, last_was_write, .. } => {
            *last_was_write = true;
            file_write_buffered(f, wbuf, *fifo, bytes)
                .map(|()| true)
                .map_err(|e| Flow::error(format!("Error writing to connection:  {}", os_error_message(&e))))
        }
        ConnKind::Sock(s) => {
            if s.write(bytes) {
                Ok(true)
            } else {
                Err(Flow::error("problem writing to connection"))
            }
        }
        // R: connections.c null_write
        ConnKind::Gz { writing: false, .. }
        | ConnKind::Url { .. }
        | ConnKind::Gzcon { writing: false, .. }
        | ConnKind::File { fp: None, .. }
        | ConnKind::Terminal
        | ConnKind::TextIn { .. }
        | ConnKind::TextOut { .. }
        | ConnKind::ServSock { .. } => Err(Flow::error("cannot write to this connection")),
        ConnKind::Native { .. } => Ok(false),
    }
}

fn i_tempdir() -> RResult<String> {
    super::sysutils::session_tempdir()
}

/// `con->close(con)`
pub(super) fn con_close_raw(i: &mut Interp, n: i32) -> RResult<()> {
    if matches!(i.conns.get(n)?.kind, ConnKind::Gzcon { .. }) {
        return gzcon_close(i, n);
    }
    let con = i.conns.get(n)?;
    // R: connections.c:con_close1 closes con->inconv
    con.drop_unread();
    if con.inconv.take().is_some() {
        con.save = -1000;
    }
    match &mut con.kind {
        ConnKind::Native { .. } => return native_bridge()?.conn_close(i, n),
        // gzcon_close, above
        ConnKind::Gzcon { .. } => {}
        ConnKind::File { fp, wbuf, .. } => {
            if let Some(mut f) = fp.take() {
                let _ = flush_file_wbuf(&mut f, wbuf);
                con.status = if f.flush().is_ok() { 0 } else { -1 };
            }
            con.isopen = false;
        }
        ConnKind::TextOut { .. } => {
            // R: connections.c:outtext_close
            let (namesym, env, line) = match &mut con.kind {
                ConnKind::TextOut { namesym, env, data, lastline } => {
                    let line = if lastline.is_empty() {
                        None
                    } else {
                        data.push(text_out_line(lastline));
                        lastline.clear();
                        Some(Value::strs(data.clone()))
                    };
                    (*namesym, env.clone(), line)
                }
                _ => unreachable!(),
            };
            if let Some(sym) = namesym {
                if env.has_local(sym) {
                    env.set_binding_locked(sym, false);
                }
                if let Some(v) = line {
                    env.set_local(sym, v);
                }
            }
        }
        ConnKind::Gz { codec, level, out, writing, append, data, xz, .. } => {
            if *writing {
                let name = expand_file_name(&con.description);
                let bytes = match codec {
                    Codec::Gzip => {
                        let mut enc =
                            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(*level as u32));
                        enc.write_all(out).is_ok().then(|| enc.finish().ok()).flatten()
                    }
                    Codec::Bzip2 => {
                        let mut enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::new(*level as u32));
                        enc.write_all(out).is_ok().then(|| enc.finish().ok()).flatten()
                    }
                    // R: connections.c:xzfile_open/xzfile_close
                    Codec::Xz { .. } => xz_encode(out, xz_preset(*level)).ok(),
                };
                let file =
                    std::fs::OpenOptions::new().write(true).create(true).append(*append).truncate(!*append).open(&name);
                con.status = match (bytes, file) {
                    (Some(b), Ok(mut f)) => {
                        if f.write_all(&b).is_ok() {
                            0
                        } else {
                            -1
                        }
                    }
                    _ => -1,
                };
            }
            out.clear();
            data.clear();
            *xz = None;
            con.isopen = false;
        }
        ConnKind::TextIn { .. } | ConnKind::Terminal | ConnKind::Raw { .. } => {}
        ConnKind::Url { data, .. } => {
            data.clear();
            con.isopen = false;
        }
        ConnKind::Sock(s) => {
            s.close();
            con.isopen = false;
        }
        ConnKind::ServSock { fd, .. } => {
            *fd = None;
            con.isopen = false;
        }
    }
    Ok(())
}

/// R: connections.c:checkClose
fn check_close(i: &mut Interp, n: i32) -> RResult<()> {
    if i.conns.get(n)?.isopen {
        con_close_raw(i, n)?;
        let st = i.conns.get(n)?.status;
        if st != NA_INTEGER && st < 0 {
            i.warning("Problem closing connection")?;
        }
    }
    Ok(())
}

/// R: connections.c:con_destroy
pub fn con_destroy(i: &mut Interp, n: i32) -> RResult<i32> {
    check_close(i, n)?;
    let st = i.conns.get(n)?.status;
    con_free(i, n)?;
    Ok(st)
}

/// `con->destroy(con)`, then the slot is freed (R: the end of
/// con_close1, and con_destroy's `free`). The caller has closed it if it
/// needed closing.
fn con_free(i: &mut Interp, n: i32) -> RResult<()> {
    // (an invalid number: the open that failed on it said so)
    if n < 0 || n as usize >= i.conns.table.len() {
        return Ok(());
    }
    let wrapped = match i.conns.table[n as usize].as_deref_mut() {
        Some(Connection { kind: ConnKind::Gzcon { inner, .. }, .. }) => inner.take(),
        _ => None,
    };
    let r = match i.conns.table[n as usize].as_deref().map(|c| &c.kind) {
        Some(ConnKind::Native { .. }) => native_bridge().and_then(|b| b.conn_destroy(i, n)),
        // R: connections.c null_destroy, text_destroy, outtext_destroy:
        // the built-in kinds own nothing beyond their `Connection`
        Some(
            ConnKind::Terminal
            | ConnKind::File { .. }
            | ConnKind::TextIn { .. }
            | ConnKind::TextOut { .. }
            | ConnKind::Gz { .. }
            | ConnKind::Url { .. }
            | ConnKind::Gzcon { .. }
            | ConnKind::Raw { .. }
            | ConnKind::Sock(_)
            | ConnKind::ServSock { .. },
        )
        | None => Ok(()),
    };
    i.conns.table[n as usize] = None;
    // a gzcon's wrapped connection (gzcon_close closed it)
    if let Some(w) = wrapped {
        let k = i.conns.hide(w);
        let _ = check_close(i, k);
        con_free(i, k)?;
    }
    r
}

fn file_fgetc(con: &mut Connection) -> i32 {
    let ConnKind::File { fp: Some(f), rbuf, rpos, last_was_write, fifo, wbuf, frpos, wpos, buffered, .. } =
        &mut con.kind
    else {
        return EOF;
    };
    // R: buff_fgetc reads con->buff before file_read switches to reading
    if *last_was_write && !(*buffered && *rpos < rbuf.len()) {
        if !*fifo {
            use std::io::{Seek, SeekFrom};
            let _ = flush_file_wbuf(f, wbuf);
            *wpos = f.stream_position().unwrap_or(*wpos);
            let _ = f.seek(SeekFrom::Start(*frpos));
            if !*buffered {
                rbuf.clear();
                *rpos = 0;
            }
        }
        *last_was_write = false;
    }
    // R: connections.c fifo_fgetc_internal — fifos read unbuffered.
    if *fifo {
        let mut b = [0u8; 1];
        return match f.read(&mut b) {
            Ok(1) => b[0] as i32,
            _ => EOF,
        };
    }
    if *rpos >= rbuf.len() {
        rbuf.resize(if *buffered { RBUFFCON_LEN } else { 8192 }, 0);
        match f.read(rbuf) {
            Ok(k) if k > 0 => {
                rbuf.truncate(k);
                *rpos = 0;
            }
            _ => {
                rbuf.clear();
                *rpos = 0;
                return EOF;
            }
        }
    }
    let c = rbuf[*rpos];
    *rpos += 1;
    c as i32
}

/// `con->fgetc_internal(con)`: without CR mapping, pushback or input
/// re-encoding.
fn con_fgetc_internal(con: &mut Connection) -> i32 {
    match &mut con.kind {
        ConnKind::TextIn { data, cur } | ConnKind::Url { data, cur, .. } => {
            if *cur >= data.len() {
                EOF
            } else {
                *cur += 1;
                data[*cur - 1] as i32
            }
        }
        ConnKind::File { .. } => file_fgetc(con),
        ConnKind::Gz { data, cur, xz, .. } => {
            if *cur < data.len() {
                *cur += 1;
                data[*cur - 1] as i32
            } else if xz.is_some() {
                // a read of a failing xz decoder, which may warn: xz_fgetc
                NATIVE_GETC
            } else {
                EOF
            }
        }
        ConnKind::Gzcon { data, cur, .. } => {
            if *cur >= data.len() {
                EOF
            } else {
                *cur += 1;
                data[*cur - 1] as i32
            }
        }
        ConnKind::Raw { data, pos, nbytes } => {
            // R: connections.c raw_fgetc
            if *pos >= *nbytes {
                EOF
            } else {
                *pos += 1;
                data[*pos - 1] as i32
            }
        }
        // R: connections.c:stdin_fgetc
        ConnKind::Terminal => crate::console::getchar(),
        ConnKind::TextOut { .. } | ConnKind::ServSock { .. } => EOF,
        // R: sockconn.c:sock_fgetc_internal
        ConnKind::Sock(s) => {
            let mut b = [0u8; 1];
            let (n, incomplete) = s.read(&mut b, con.blocking);
            con.incomplete = incomplete;
            if n == 1 { b[0] as i32 } else { EOF }
        }
        ConnKind::Native { .. } => NATIVE_GETC,
    }
}

/// R: connections.c:Rconn_fgetc: the next character of connection `n`,
/// CR and CRLF read as LF, pushed-back lines first.
pub(super) fn rconn_fgetc(i: &mut Interp, n: i32) -> RResult<i32> {
    // pushed-back lines and a saved character come first, whatever the kind
    let c = rconn_fgetc_builtin(i.conns.get(n)?);
    if c != NATIVE_GETC {
        return Ok(c);
    }
    native_fgetc(i, n)
}

/// Rconn_fgetc's call of a native connection's `con->fgetc` (its own, or
/// dummy_fgetc), with CR and CRLF read as LF; and of `dummy_fgetc` for a
/// built-in connection that re-encodes its input.
#[cold]
#[inline(never)]
fn native_fgetc(i: &mut Interp, n: i32) -> RResult<i32> {
    if i.conns.get(n)?.inconv.is_some() {
        return rconn_fgetc_conv(i, n);
    }
    if matches!(i.conns.get(n)?.kind, ConnKind::Gz { .. }) {
        return xz_fgetc(i, n);
    }
    let bridge = native_bridge()?;
    let c = bridge.conn_fgetc(i, n)?;
    if c == '\r' as i32 {
        let c2 = bridge.conn_fgetc(i, n)?;
        if c2 != '\n' as i32 {
            i.conns.get(n)?.save = if c2 != '\r' as i32 { c2 } else { '\n' as i32 };
            return Ok('\n' as i32);
        }
        return Ok(c2);
    }
    Ok(c)
}

/// [`rconn_fgetc`] for a connection of a built-in kind, which loops hold
/// borrowed: no table lookup per character. A connection that re-encodes
/// its input gives [`NATIVE_GETC`] (its `save` is [`SAVE_CONV`]), and
/// [`rconn_fgetc`] reads it through `dummy_fgetc`: the other connections
/// pay nothing per character for it.
// R: connections.c:Rconn_fgetc
#[inline]
pub(super) fn rconn_fgetc_builtin(con: &mut Connection) -> i32 {
    if con.pushback.is_empty() {
        if con.save != -1000 {
            return saved_getc(con);
        }
        let c = con_fgetc_internal(con);
        if c == '\r' as i32 {
            let c2 = con_fgetc_internal(con);
            if c2 == NATIVE_GETC {
                // the character after the CR is a read of a failing xz
                // decoder: xz_fgetc
                con.save = SAVE_CR;
                return NATIVE_GETC;
            }
            if c2 != '\n' as i32 {
                con.save = if c2 != '\r' as i32 { c2 } else { '\n' as i32 };
                return '\n' as i32;
            }
            return c2;
        }
        return c;
    }
    pushback_getc(con)
}

/// Rconn_fgetc's read of an xzfile connection with a failing decoder
/// ([`XzLive`]) once its buffer is empty: a text connection's buffer fill
/// (`buff_fgetc`), else one byte (`xzfile_fgetc_internal`); and the
/// warning of that `xzfile_read`.
// R: connections.c:buff_fgetc, buff_fill, xzfile_fgetc_internal
#[cold]
#[inline(never)]
fn xz_getc(con: &mut Connection) -> (i32, Option<String>) {
    let text = con.text;
    let ConnKind::Gz { data, cur, xz: Some(x), .. } = &mut con.kind else {
        unreachable!("xz_getc on a connection without an xz reader")
    };
    if !text {
        let mut b = [0u8; 1];
        let (k, warn) = x.read(&mut b);
        return (if k == 1 { b[0] as i32 } else { EOF }, warn);
    }
    data.clear();
    data.resize(RBUFFCON_LEN_DEFAULT, 0);
    let (k, warn) = x.read(data);
    data.truncate(k);
    *cur = usize::from(k > 0);
    (data.first().map_or(EOF, |&b| b as i32), warn)
}

/// [`xz_getc`] for an input conversion's fill (`dummy_fgetc`), which has
/// no interpreter: the warning waits in [`XzLive::queued`].
#[cold]
#[inline(never)]
fn xz_fgetc_queued(con: &mut Connection) -> i32 {
    let (c, warn) = xz_getc(con);
    if let (Some(w), ConnKind::Gz { xz: Some(x), .. }) = (warn, &mut con.kind) {
        x.queued.push(w);
    }
    c
}

/// Rconn_fgetc at a read of an xzfile connection with a failing decoder
/// ([`xz_getc`]), which warns; CR and CRLF read as LF.
// R: connections.c:Rconn_fgetc
#[cold]
#[inline(never)]
fn xz_fgetc(i: &mut Interp, n: i32) -> RResult<i32> {
    let con = i.conns.get(n)?;
    let after_cr = con.save == SAVE_CR;
    if after_cr {
        con.save = -1000;
    }
    let (c, warn) = xz_getc(con);
    if let Some(w) = warn {
        i.warning(w)?;
    }
    let con = i.conns.get(n)?;
    if after_cr {
        // the character after a CR
        if c != '\n' as i32 {
            con.save = if c != '\r' as i32 { c } else { '\n' as i32 };
        }
        return Ok('\n' as i32);
    }
    if c == '\r' as i32 {
        let c2 = con_fgetc_internal(con);
        if c2 == NATIVE_GETC {
            con.save = SAVE_CR;
            return xz_fgetc(i, n);
        }
        if c2 != '\n' as i32 {
            con.save = if c2 != '\r' as i32 { c2 } else { '\n' as i32 };
            return Ok('\n' as i32);
        }
        return Ok(c2);
    }
    Ok(c)
}

/// [`rconn_fgetc_builtin`] when `con->save` holds something: bytes a parse
/// left unread, the mark of a re-encoding connection, or a character saved
/// by CR/LF handling. Out of line so that the per-character path stays small.
#[cold]
#[inline(never)]
fn saved_getc(con: &mut Connection) -> i32 {
    match con.save {
        SAVE_UNREAD => unread_getc(con).unwrap_or(EOF),
        SAVE_CONV | SAVE_CR => NATIVE_GETC,
        c => {
            con.save = -1000;
            c
        }
    }
}

/// The next byte of the pushed-back lines.
#[inline]
fn pushback_getc(con: &mut Connection) -> i32 {
    let last = con.pushback.len() - 1;
    let c = con.pushback[last][con.pos_pushback] as i32;
    con.pos_pushback += 1;
    if con.pos_pushback >= con.pushback[last].len() {
        con.pushback.pop();
        con.pos_pushback = 0;
    }
    c
}

/// [`rconn_fgetc`] for a connection that re-encodes its input, with
/// `dummy_fgetc` as `con->fgetc` and the CR/LF state in [`InConv::save`];
/// `dummy_fgetc` may warn.
// R: connections.c:Rconn_fgetc
#[cold]
#[inline(never)]
fn rconn_fgetc_conv(i: &mut Interp, n: i32) -> RResult<i32> {
    let con = i.conns.get(n)?;
    let c = if !con.pushback.is_empty() {
        pushback_getc(con)
    } else if let Some(c) = unread_getc(con) {
        c
    } else {
        let saved = con.inconv.as_mut().map_or(-1000, |ic| std::mem::replace(&mut ic.save, -1000));
        if saved != -1000 {
            saved
        } else {
            let c = dummy_fgetc(con);
            if c == '\r' as i32 {
                let c2 = dummy_fgetc(con);
                if c2 != '\n' as i32 {
                    if let Some(ic) = con.inconv.as_mut() {
                        ic.save = if c2 != '\r' as i32 { c2 } else { '\n' as i32 };
                    }
                    '\n' as i32
                } else {
                    c2
                }
            } else {
                c
            }
        }
    };
    if let ConnKind::Gz { xz: Some(x), .. } = &mut con.kind
        && !x.queued.is_empty()
    {
        for msg in std::mem::take(&mut x.queued) {
            i.warning(msg)?;
        }
    }
    let con = i.conns.get(n)?;
    if con.inconv.as_ref().is_some_and(|ic| ic.invalid) {
        warn_invalid_input(i, n)?;
    }
    Ok(c)
}

/// Write text to connection `n` (R's `Rconn_printf(con, "%s", s)`).
/// `stdout()` writes to the process's stdout, whatever the sinks
/// (R: connections.c:stdout_vfprintf); `stderr()` writes where messages go,
/// the connection of `sink(type = "message")` if there is one
/// (stderr_vfprintf calls REvprintf_internal, which follows R_ErrorCon).
#[inline(always)]
pub fn conn_write(i: &mut Interp, n: i32, text: &str) -> RResult<()> {
    if n == 2 && i.conns.error_con != 2 {
        stderr_sunk(i, text);
        return Ok(());
    }
    conn_print(i, n, text)
}

/// `stderr()`'s writes under `sink(type = "message")`, out of line.
#[cold]
#[inline(never)]
fn stderr_sunk(i: &mut Interp, text: &str) {
    reprintf_sink(i, text);
}

/// conn_write once `stderr()` is the terminal: the terminals write to the
/// process streams directly.
fn conn_print(i: &mut Interp, n: i32, text: &str) -> RResult<()> {
    let bytes = crate::locale::output_bytes(text);
    match n {
        1 => {
            let _ = i.out.write_all(&bytes);
            return Ok(());
        }
        2 => {
            let _ = i.out.flush();
            let _ = i.err.write_all(&bytes);
            return Ok(());
        }
        _ => {}
    }
    let con = i.conns.get(n)?;
    let mut raw_bytes: Option<Vec<u8>> = None;
    file_switch_to_write(&mut con.kind);
    match &mut con.kind {
        ConnKind::Native { .. } => return native_bridge()?.conn_vfprintf(i, n, text),
        ConnKind::Terminal | ConnKind::TextIn { .. } => return Err(Flow::error("cannot write to this connection")),
        // R: scan.c's writeLines goes through con->write (bytes); the
        // gzcon buffers them for its deflate stream.
        ConnKind::Gzcon { out, writing: true, .. } => {
            out.extend_from_slice(&bytes);
            Ok(())
        }
        ConnKind::Gzcon { writing: false, .. } => return Err(Flow::error("cannot write to this connection")),
        ConnKind::Gz { out, writing, .. } => {
            if !*writing {
                return Err(Flow::error("cannot write to this connection"));
            }
            out.extend_from_slice(&bytes);
            return Ok(());
        }
        // Rconn_printf on a raw connection formats then calls con->write
        // (connections.c dummy_vfprintf).
        ConnKind::Raw { .. } | ConnKind::Sock(_) => {
            raw_bytes = Some(bytes.to_vec());
            Ok(())
        }
        ConnKind::Url { .. } => Err(Flow::error("cannot write to this connection")),
        ConnKind::ServSock { .. } => Err(Flow::error("cannot write to this connection")),
        ConnKind::File { fp, wbuf, fifo, last_was_write, .. } => match fp {
            Some(f) => {
                *last_was_write = true;
                if let Err(e) = file_write_buffered(f, wbuf, *fifo, &bytes) {
                    return Err(Flow::error(format!("Error writing to connection:  {}", os_error_message(&e))));
                }
                Ok(())
            }
            None => Err(Flow::error("cannot write to this connection")),
        },
        ConnKind::TextOut { .. } => {
            // R: connections.c:text_vfprintf
            let ConnKind::TextOut { namesym, env, data, lastline } = &mut con.kind else { unreachable!() };
            lastline.extend_from_slice(&bytes);
            let mut changed = false;
            while let Some(pos) = lastline.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = lastline.drain(..=pos).collect();
                data.push(text_out_line(&line[..line.len() - 1]));
                changed = true;
            }
            con.incomplete = !lastline.is_empty();
            if changed {
                if let Some(sym) = *namesym {
                    let env = env.clone();
                    let v = Value::strs(data.clone());
                    if env.has_local(sym) {
                        env.set_binding_locked(sym, false);
                    }
                    env.set_local(sym, v);
                    env.set_binding_locked(sym, true);
                }
            }
            Ok(())
        }
    }?;
    if let Some(bytes) = raw_bytes {
        con_write_bytes(i, n, &bytes)?;
    }
    Ok(())
}

/// A line of a text connection: its bytes as written, marked UTF-8 in a
/// UTF-8 locale and latin1 in a latin1 one, native otherwise.
// R: connections.c:mkCharLocal
fn text_out_line(line: &[u8]) -> RStr {
    use crate::value::{Encoding, RString};
    let enc = if crate::locale::utf8locale() {
        Encoding::Utf8
    } else if crate::locale::latin1locale() {
        Encoding::Latin1
    } else {
        Encoding::Native
    };
    Some(RString::from_bytes(line, enc))
}

fn conn_flush(i: &mut Interp, n: i32) -> RResult<()> {
    match n {
        1 => {
            let _ = i.out.flush();
        }
        2 => {
            let _ = i.err.flush();
        }
        _ => con_fflush(i, n)?,
    }
    Ok(())
}

/// `con->fflush(con)`.
fn con_fflush(i: &mut Interp, n: i32) -> RResult<()> {
    let con = i.conns.get(n)?;
    match &mut con.kind {
        ConnKind::File { fp: Some(f), wbuf, .. } => {
            let _ = flush_file_wbuf(f, wbuf);
            let _ = f.flush();
        }
        // R: connections.c null_fflush
        ConnKind::File { fp: None, .. }
        | ConnKind::Terminal
        | ConnKind::TextIn { .. }
        | ConnKind::TextOut { .. }
        | ConnKind::Gz { .. }
        | ConnKind::Url { .. }
        | ConnKind::Gzcon { .. }
        | ConnKind::Raw { .. }
        | ConnKind::Sock(_)
        | ConnKind::ServSock { .. } => {}
        ConnKind::Native { .. } => {
            native_bridge()?.conn_fflush(i, n)?;
        }
    }
    Ok(())
}

/// `Rprintf` while a sink is active (R: printutils.c:Rvprintf).
pub fn rprintf_sink(i: &mut Interp, text: &str) {
    let mut con_num = i.conns.output_con;
    let mut k = 0;
    loop {
        if conn_write(i, con_num, text).is_err() {
            break;
        }
        con_num = i.conns.active_sink(k);
        k += 1;
        if con_num <= 0 {
            break;
        }
    }
}

/// `REprintf` while the message connection is diverted: written, then
/// flushed (R: printutils.c:REvprintf_internal).
pub fn reprintf_sink(i: &mut Interp, text: &str) {
    let n = i.conns.error_con;
    if conn_write(i, n, text).is_err() {
        i.conns.error_con = 2;
        let _ = i.err.write_all(&crate::locale::output_bytes(text));
        return;
    }
    let _ = conn_flush(i, n);
}

/// R: connections.c:switch_or_tee_stdout. Returns whether the sink stack
/// changed.
fn switch_or_tee_stdout(i: &mut Interp, icon: i32, close_on_exit: bool, tee: bool) -> RResult<bool> {
    if icon == i.conns.output_con {
        return Ok(false);
    }
    if icon >= 0 && i.conns.sink_number >= NSINKS - 1 {
        return Err(Flow::error("sink stack is full"));
    }
    if icon == 0 {
        return Err(Flow::error("cannot switch output to stdin"));
    } else if icon == 1 || icon == 2 {
        let st = &mut i.conns;
        st.sink_number += 1;
        st.sink_cons[st.sink_number] = icon;
        st.output_con = icon;
        st.sink_split[st.sink_number] = tee;
        st.sink_close[st.sink_number] = 0;
    } else if icon >= 3 {
        let mut toclose = 2 * close_on_exit as i32;
        let con = i.conns.get(icon)?;
        if !con.isopen {
            let mode = std::mem::replace(&mut con.mode, "wt".into());
            let ok = con_open(i, icon)?;
            i.conns.get(icon)?.mode = mode;
            if !ok {
                return Err(Flow::error("cannot open the connection"));
            }
            if !i.conns.get(icon)?.canwrite {
                con_close_raw(i, icon)?;
                return Err(Flow::error("cannot write to this connection"));
            }
            toclose = 1;
        } else if !con.canwrite {
            return Err(Flow::error("cannot write to this connection"));
        }
        let st = &mut i.conns;
        st.sink_number += 1;
        st.sink_cons[st.sink_number] = icon;
        st.output_con = icon;
        st.sink_close[st.sink_number] = toclose;
        st.sink_split[st.sink_number] = tee;
    } else {
        // removing a sink
        if i.conns.sink_number == 0 {
            i.warning("no sink to remove")?;
            return Ok(false);
        }
        let st = &mut i.conns;
        st.sink_number -= 1;
        st.output_con = st.sink_cons[st.sink_number];
        let icon = st.sink_cons[st.sink_number + 1];
        let close = st.sink_close[st.sink_number + 1];
        if icon >= 3 {
            if close == 1 {
                check_close(i, icon)?;
            } else if close == 2 {
                con_destroy(i, icon)?;
            }
        }
    }
    Ok(true)
}

fn connection_value(con: &Connection, n: usize) -> Value {
    let mut v = Value::int(n as i32);
    v.set_attr_raw(s::CLASS, Value::strs_from([con.class.as_str(), "connection"]));
    if n > 2 {
        let id = match &con.kind {
            ConnKind::Native { .. } => native_bridge().map(|b| b.conn_id_value(n as i32)).unwrap_or(Value::Null),
            _ => con.ex_ptr.clone(),
        };
        if !id.is_null() {
            v.set_attr_raw(Sym::new("conn_id"), id);
        }
    }
    v
}

/// R: connections.c:NextConnection: the first free slot from 3 on; when
/// none is, a collection (which closes the connections nothing refers
/// to) and a second look.
pub fn next_connection(i: &mut Interp) -> RResult<usize> {
    if let Ok(n) = i.conns.next_connection() {
        return Ok(n);
    }
    // Try to reclaim unused ones
    super::misc::r_gc(i)?;
    i.conns.next_connection()
}

/// Close the connections of built-in kinds whose `conn_id` nothing but
/// the table refers to, as GNU R's collector does by finalizing it
/// (conFinalizer); a sink keeps its connection (do_sink preserves its
/// `conn_id`). A native connection's `conn_id` is a C object with that
/// finalizer itself. Called by `gc()` after the collection.
// R: connections.c:conFinalizer
pub fn close_unused_connections(i: &mut Interp) -> RResult<()> {
    let st = &i.conns;
    let sunk = |n: usize| (1..=st.sink_number).any(|k| st.sink_cons[k] == n as i32) || st.error_con == n as i32;
    let mut dead: Vec<(usize, i32)> = Vec::new();
    for n in 3..NCONNECTIONS {
        let Some(con) = st.table[n].as_deref() else { continue };
        let Value::ExtPtr(p) = &con.ex_ptr else { continue };
        if con.is_native() || Rc::strong_count(p) > 1 || sunk(n) {
            continue;
        }
        dead.push((con.id, n as i32));
    }
    // the collector runs finalizers newest first (R_weak_refs is a stack)
    dead.sort_by_key(|d| std::cmp::Reverse(d.0));
    for (_, n) in dead {
        finalize_connection(i, n)?;
    }
    Ok(())
}

/// conFinalizer once it has found the connection: destroy it, and warn
/// if it was open (a text connection never warns).
// R: connections.c:conFinalizer
pub fn finalize_connection(i: &mut Interp, n: i32) -> RResult<()> {
    let warn = {
        let con = i.conns.get(n)?;
        (con.isopen && con.class != "textConnection")
            .then(|| format!("closing unused connection {} ({})", n, con.description))
    };
    con_destroy(i, n)?;
    if let Some(msg) = warn {
        // warning(): against the gc() that ran the finalizer
        i.warning(msg)?;
    }
    Ok(())
}

/// The connection from 3 on whose `con->id` is `id` (conFinalizer's
/// search).
pub fn connection_with_id(i: &Interp, id: usize) -> Option<i32> {
    (3..NCONNECTIONS).find(|&n| i.conns.table[n].as_deref().is_some_and(|c| c.id == id)).map(|n| n as i32)
}

/// R: connections.c:R_new_custom_connection's `Connections[ncon] = new`:
/// slot `n` (from [`next_connection`]) holds a native connection over the
/// C `struct Rconn` at `con`, whose `con->id` is `id` (from
/// [`next_connection_id`]). The bridge copies the struct's fields in.
pub fn install_native_connection(i: &mut Interp, n: i32, class: &str, description: &str, con: usize, id: usize) {
    assert!(
        (3..NCONNECTIONS as i32).contains(&n) && i.conns.table[n as usize].is_none(),
        "install_native_connection: slot {n} is not a free connection slot"
    );
    // init_con took `id` from current_id already: take no second one
    let c = Connection::with_id(class, description, "", ConnKind::Native { con }, id);
    i.conns.table[n as usize] = Some(Box::new(c));
}

/// init_con's `current_id`: the next connection id (never 0).
pub fn next_connection_id() -> usize {
    CURRENT_ID.with(|c| {
        let id = c.get().wrapping_add(1).max(1);
        c.set(id);
        id
    })
}

/// The C `struct Rconn` of a native connection.
pub fn native_struct_of(c: &Connection) -> Option<usize> {
    match c.kind {
        ConnKind::Native { con } => Some(con),
        _ => None,
    }
}

/// R: connections.c `R_MakeExternalPtr(con->id, install("connection"), R_NilValue)`
fn new_ex_ptr(id: usize) -> Value {
    let payload: Rc<dyn std::any::Any> = Rc::new(crate::value::ExtPtrAddr(id));
    Value::ExtPtr(Rc::new(ExtPtr { tag: Value::sym("connection"), attrs: Default::default(), payload: Some(payload) }))
}

/// Register `con` in the next free slot: its R object and number.
pub(super) fn install_new(i: &mut Interp, con: Connection) -> RResult<(Value, i32)> {
    let n = next_connection(i)?;
    Ok((install_connection(i, n, con), n as i32))
}

/// R: connections.c:checked_open, of the connection object `v`.
pub(super) fn con_open_new(i: &mut Interp, v: &Value) -> RResult<()> {
    checked_open(i, con_arg(v)?)
}

/// Open the new connection `n`, destroying it if the open fails.
// R: connections.c:checked_open
fn checked_open(i: &mut Interp, n: i32) -> RResult<()> {
    match con_open(i, n) {
        Ok(true) => Ok(()),
        Ok(false) => {
            con_free(i, n)?;
            Err(Flow::error("cannot open the connection"))
        }
        Err(e) => {
            // R: connections.c:cend_con_destroy — the context's cleanup
            // destroys the connection while the error unwinds. An open
            // that already destroyed it (set_iconv_error) makes that
            // con_destroy's getConnection() fail, and its error replaces
            // the original one.
            if i.conns.table.get(n as usize).is_none_or(|c| c.is_none()) {
                return Err(Flow::error("invalid connection"));
            }
            con_free(i, n)?;
            Err(e)
        }
    }
}

/// Whether `n` is a live stream, which `con_read_all` would wait on.
pub fn is_socket(i: &mut Interp, n: i32) -> RResult<bool> {
    Ok(matches!(i.conns.get(n)?.kind, ConnKind::Sock(_)))
}

/// Register a new connection in slot `n` and return its R object.
fn install_connection(i: &mut Interp, n: usize, mut con: Connection) -> Value {
    con.ex_ptr = new_ex_ptr(con.id);
    let v = connection_value(&con, n);
    i.conns.table[n] = Some(Box::new(con));
    v
}

fn inherits_connection(v: &Value) -> bool {
    v.inherits("connection")
}

fn con_arg(v: &Value) -> RResult<i32> {
    if !inherits_connection(v) {
        return Err(Flow::error("'con' is not a connection"));
    }
    Ok(as_integer(v))
}

fn str1_arg(v: &Value, what: &str) -> RResult<String> {
    match v {
        Value::Str(s) if s.data.len() == 1 => Ok(s.data[0].as_deref().unwrap_or("NA").to_string()),
        _ => Err(Flow::error(format!("invalid '{}' argument", what))),
    }
}

// ----------------------------------------------------------------------
// Builtins

fn do_stdin(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let con = i.conns.get(0)?;
    Ok(connection_value(con, 0))
}

fn do_stdout(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = i.conns.output_con;
    let con = i.conns.get(n)?;
    let mut v = Value::int(n);
    v.set_attr_raw(s::CLASS, Value::strs_from([con.class.as_str(), "connection"]));
    Ok(v)
}

fn do_stderr(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let con = i.conns.get(2)?;
    let mut v = Value::int(2);
    v.set_attr_raw(s::CLASS, Value::strs_from([con.class.as_str(), "connection"]));
    Ok(v)
}

fn do_isatty(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let con = as_integer(&arg(&args, 0));
    use std::io::IsTerminal;
    Ok(Value::lgl(match con {
        0 => std::io::stdin().is_terminal(),
        1 => std::io::stdout().is_terminal(),
        2 => std::io::stderr().is_terminal(),
        _ => false,
    }))
}

/// Check for a compressed file (R: connections.c:comp_type_from_file).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Compression {
    Gzip,
    Bzip2,
    /// `alone`: the legacy .lzma format (R's `subtype` 1)
    Xz {
        alone: bool,
    },
    Zstd,
    Lzop,
}

/// R: connections.c:comp_type_from_memory with `with_zlib = FALSE` (what
/// `do_file` passes through `comp_type_from_file`). The magic numbers are
/// checked beyond their short prefix so that an uncompressed file is not
/// mistaken for a compressed one (R's PR#18768 fixed that for bzip2).
fn compressed_file_type(path: &str) -> Option<Compression> {
    let mut buf = [0u8; 10];
    let Ok(mut f) = File::open(path) else { return None };
    let len = f.read(&mut buf).unwrap_or(0);
    if len >= 2 && buf[0] == 0x1f && buf[1] == 0x8b {
        return Some(Compression::Gzip);
    }
    if len >= 10 && &buf[..3] == b"BZh" && (b'1'..=b'9').contains(&buf[3]) {
        // 0x314159265359 (BCD pi) or 0x177245385090 (BCD sqrt(pi)).
        if &buf[4..10] == b"\x31\x41\x59\x26\x53\x59" || &buf[4..10] == b"\x17\x72\x45\x38\x50\x90" {
            return Some(Compression::Bzip2);
        }
    }
    if len >= 5 && buf[0] == 0xFD && &buf[1..5] == b"7zXZ" {
        return Some(Compression::Xz { alone: false });
    }
    if len >= 5 && (buf[0] == 0xFF && &buf[1..5] == b"LZMA" || buf[..5] == [b']', b'\0', b'\0', 0x80, b'\0']) {
        return Some(Compression::Xz { alone: true });
    }
    if len >= 4 && buf[0] == 0x89 && &buf[1..4] == b"LZO" {
        return Some(Compression::Lzop);
    }
    if len >= 4 && buf[..4] == [0x28, 0xB5, 0x2F, 0xFD] {
        return Some(Compression::Zstd);
    }
    None
}

/// `.Internal(file(description, open, blocking, encoding, method, raw))`
/// (R: connections.c:do_url with op = 1).
fn do_file(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let url = match arg(&args, 0) {
        Value::Str(s) if s.data.len() == 1 && s.data[0].is_some() => s.data[0].clone().unwrap().to_string(),
        _ => return Err(Flow::error("invalid 'description' argument")),
    };
    let mut open = str1_arg(&arg(&args, 1), "open")?;
    let block = as_logical(&arg(&args, 2));
    if block == NA_LOGICAL {
        return Err(Flow::error("invalid 'blocking' argument"));
    }
    let enc = match arg(&args, 3) {
        Value::Str(s) if s.data.len() == 1 && s.data[0].as_deref().map(|x| x.len() <= 100).unwrap_or(true) => {
            s.data[0].as_deref().unwrap_or("NA").to_string()
        }
        _ => return Err(Flow::error("invalid 'encoding' argument")),
    };
    let raw = as_logical(&arg(&args, 5));
    if raw == NA_LOGICAL {
        return Err(Flow::error_call(call.clone(), "NA in coercion to boolean"));
    }
    let mut raw = raw != 0;
    if url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("ftp://")
        || url.starts_with("ftps://")
    {
        return Err(Flow::error("robigo: URL connections are not implemented yet"));
    }
    let ncon = next_connection(i)?;
    let mut comp: Option<Codec> = None;
    let path = url.strip_prefix("file://").map(|s| s.to_string());
    let description = path.clone().unwrap_or_else(|| url.clone());
    if path.is_none() {
        if url.is_empty() {
            if open.is_empty() {
                open = "w+".into();
            }
            if open != "w+" && open != "w+b" {
                open = "w+".into();
                i.warning("file(\"\") only supports open = \"w+\" and open = \"w+b\": using the former")?;
            }
        }
        if url == "clipboard" || url == "X11_primary" || url == "X11_secondary" || url == "X11_clipboard" {
            return Err(Flow::error("robigo: clipboard connections are not implemented yet"));
        }
        let efn = expand_file_name(&url);
        if !raw && !url.is_empty() {
            if let Ok(md) = std::fs::metadata(&efn) {
                use std::os::unix::fs::FileTypeExt;
                if md.file_type().is_fifo() {
                    raw = true;
                    i.warning(format!("using 'raw = TRUE' because '{}' is a fifo or pipe", url))?;
                } else if !md.is_file() && efn != "/dev/null" {
                    i.warning(format!("'raw = FALSE' but '{}' is not a regular file", url))?;
                }
            }
        }
        // R: connections.c:do_file — a compressed file read in text mode is
        // opened through its gz/bz/xz/zstd connection; only its class
        // attribute stays "file".
        if !raw && (open.is_empty() || open == "r" || open == "rt") {
            match compressed_file_type(&efn) {
                Some(Compression::Gzip) => comp = Some(Codec::Gzip),
                Some(Compression::Bzip2) => comp = Some(Codec::Bzip2),
                Some(Compression::Xz { alone }) => comp = Some(Codec::Xz { alone }),
                Some(Compression::Zstd) => {
                    return Err(Flow::error("robigo: zstd compressed file connections are not implemented yet"));
                }
                // R: connections.c:comp_type_from_memory — lzop is not
                // supported by R either.
                Some(Compression::Lzop) => {
                    return Err(Flow::error("this is a lzop-compressed file which this build of R does not support"));
                }
                None => {}
            }
        }
    }
    let mode = if open.is_empty() {
        // R: newgzfile/newbzfile(url, strlen(open) ? open : "rt", compress)
        if comp.is_some() { "rt".to_string() } else { "r".to_string() }
    } else {
        open.clone()
    };
    let kind = if let Some(codec) = comp {
        ConnKind::Gz {
            codec,
            level: 0,
            data: Vec::new(),
            cur: 0,
            out: Vec::new(),
            writing: false,
            append: false,
            xz: None,
        }
    } else {
        ConnKind::File {
            raw,
            fp: None,
            rbuf: Vec::new(),
            rpos: 0,
            wbuf: Vec::new(),
            temp: false,
            last_was_write: false,
            frpos: 0,
            wpos: 0,
            fifo: false,
            buffered: false,
        }
    };
    // R: the connection is a gzfile/bzfile one (con->class, which summary(),
    // getConnection() and gzcon() see); only the returned value's class
    // attribute is "file" (class2).
    let mut con = Connection::new(comp.map_or("file", Codec::class), &description, &mode, kind);
    // gzfile connections decompress on open and cannot seek in robigo.
    con.canseek = !raw && comp.is_none();
    con.blocking = block != 0;
    con.encname = enc.chars().take(99).collect();
    if !con.encname.is_empty() && con.encname != "native.enc" {
        con.canseek = false;
    }
    let mut v = install_connection(i, ncon, con);
    if comp.is_some() {
        v.set_attr_raw(s::CLASS, Value::strs_from(["file", "connection"]));
    }
    if !open.is_empty() {
        checked_open(i, ncon as i32)?;
    }
    Ok(v)
}

/// `.Internal(url(description, open, blocking, encoding, method, headers))`.
// R: connections.c:do_url — `file://` descriptions open through the file
// connection (class "file"); http/https/ftp(s) through libcurl (the
// connection class "url-libcurl", the object class "url"), fetched into
// memory when opened.
fn do_url(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let url = match arg(&args, 0) {
        Value::Str(s) if s.data.len() == 1 && s.data[0].is_some() => s.data[0].clone().unwrap().to_string(),
        _ => return Err(Flow::error("invalid 'description' argument")),
    };
    let inet = ["http://", "ftp://", "https://", "ftps://"].iter().any(|p| url.starts_with(p));
    let open = match arg(&args, 1) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error("invalid 'open' argument")),
    };
    let block = as_logical(&arg(&args, 2));
    if block == NA_LOGICAL {
        return Err(Flow::error("invalid 'blocking' argument"));
    }
    let enc = match arg(&args, 3) {
        Value::Str(s) if s.data.len() == 1 && s.data[0].as_deref().unwrap_or("NA").len() <= 100 => {
            s.data[0].as_deref().unwrap_or("NA").to_string()
        }
        _ => return Err(Flow::error("invalid 'encoding' argument")),
    };
    let cmeth = crate::coerce::as_string(&arg(&args, 4)).as_deref().unwrap_or("NA").to_string();
    let defmeth = cmeth == "default";
    let meth = cmeth == "libcurl" || defmeth;
    if cmeth == "wininet" {
        return Err(Flow::error("method = \"wininet\" is only supported on Windows"));
    }
    // headers: NULL or list(<"name: value" lines>, <flat>)
    let headers: Vec<String> = match arg(&args, 5) {
        Value::List(l) => match l.data.first() {
            Some(Value::Str(h)) => h.data.iter().map(|x| x.as_deref().unwrap_or("NA").to_string()).collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    if !meth {
        if url.starts_with("ftps://") {
            return Err(Flow::error("ftps:// URLs are not supported by this method"));
        }
        if url.starts_with("https://") {
            return Err(Flow::error("https:// URLs are not supported by the \"internal\" method"));
        }
    }
    if url.starts_with("file://") {
        // newfile(url + 7, ...), which do_file does for a `file://`
        // description; its arguments are (description, open, blocking,
        // encoding, method, raw).
        args.truncate(5);
        while args.len() < 5 {
            args.push(Arg::pos(Value::Null));
        }
        args.push(Arg::pos(Value::lgl_raw(0)));
        return do_file(i, call, args, env);
    }
    if !inet {
        return Err(Flow::error("URL scheme unsupported by this method"));
    }
    if !meth {
        return Err(Flow::error("the 'internal' method of url() is defunct for http:// and ftp:// URLs"));
    }
    let ncon = next_connection(i)?;
    let mode = if open.is_empty() { "r" } else { open.as_str() };
    let kind = ConnKind::Url { data: Vec::new(), cur: 0, headers };
    let mut con = Connection::new("url-libcurl", &url, mode, kind);
    con.canwrite = false;
    con.blocking = block != 0;
    con.encname = enc.chars().take(99).collect();
    if !con.encname.is_empty() && con.encname != "native.enc" {
        con.canseek = false;
    }
    let mut v = install_connection(i, ncon, con);
    v.set_attr_raw(s::CLASS, Value::strs_from(["url", "connection"]));
    if !open.is_empty() {
        checked_open(i, ncon as i32)?;
    }
    Ok(v)
}

/// `fifo(description, open, blocking, encoding)`.
// R: connections.c do_fifo
fn do_fifo(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let sfile = match arg(&args, 0) {
        Value::Str(s) if s.data.len() == 1 && s.data[0].is_some() => s.data[0].clone().unwrap().to_string(),
        _ => return Err(Flow::error("invalid 'description' argument")),
    };
    let mut open = match arg(&args, 1) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error("invalid 'open' argument")),
    };
    let block = super::character::as_bool2(&arg(&args, 2), call)?;
    let enc = match arg(&args, 3) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error("invalid 'encoding' argument")),
    };
    if enc.len() > 100 {
        return Err(Flow::error("invalid 'encoding' argument"));
    }
    if sfile.is_empty() {
        if open.is_empty() {
            open = "w+".to_string();
        }
        if open != "w+" && open != "w+b" {
            open = "w+".to_string();
            i.warning("fifo(\"\") only supports open = \"w+\" and open = \"w+b\": using the former")?;
        }
    }
    let ncon = next_connection(i)?;
    let mode = if open.is_empty() { "r".to_string() } else { open.clone() };
    let kind = ConnKind::File {
        raw: false,
        fp: None,
        rbuf: Vec::new(),
        rpos: 0,
        wbuf: Vec::new(),
        temp: false,
        last_was_write: false,
        frpos: 0,
        wpos: 0,
        fifo: true,
        buffered: false,
    };
    let mut con = Connection::new("fifo", &sfile, &mode, kind);
    con.blocking = block;
    con.encname = enc;
    let v = install_connection(i, ncon, con);
    if !open.is_empty() {
        checked_open(i, ncon as i32)?;
    }
    Ok(v)
}

/// `gzfile()`; `bzfile()`, `xzfile()` and `zstdfile()` share the entry
/// point (R's PRIMVAL); gzip, bzip2 and xz are implemented.
// R: connections.c:do_gzfile
/// `memCompress(from, type)`: `from` is a raw vector by the time the
/// base wrapper calls the internal; the type codes are the wrapper's
/// (`none` 1, `gzip` 2, `bzip2` 3, `xz` 4, `zstd` 5).
// R: connections.c:do_memCompress
fn do_mem_compress(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let from = arg(&args, 0);
    let Value::Raw(raw) = &from else {
        return Err(Flow::error("'from' must be raw or character"));
    };
    match as_integer(&arg(&args, 1)) {
        1 => Ok(from),
        // zlib's compress(): the zlib format at its default level
        2 => {
            let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
            enc.write_all(&raw.data)
                .and_then(|_| enc.finish())
                .map(Value::raws)
                .map_err(|_| Flow::error("internal error in memCompress"))
        }
        // bzlib at its maximum block size, as BZ2_bzBuffToBuffCompress(…, 9, 0, 0)
        3 => {
            let mut enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::new(9));
            enc.write_all(&raw.data)
                .and_then(|_| enc.finish())
                .map(Value::raws)
                .map_err(|_| Flow::error("internal error in memCompress"))
        }
        // one .xz stream at preset 9 | LZMA_PRESET_EXTREME
        4 => xz_encode(&raw.data, xz_preset(-9))
            .map(Value::raws)
            .map_err(|ret| Flow::error(format!("internal error {ret} in memCompress"))),
        5 => Err(Flow::error("robigo: Zstd compression is not implemented yet")),
        // R's switch default: an unknown type returns the input
        _ => Ok(from),
    }
}

/// `memDecompress(from, type)`: the wrapper's type codes, with 5 for
/// `unknown` (detected from the data).
// R: connections.c:do_memDecompress
fn do_mem_decompress(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let from = arg(&args, 0);
    let Value::Raw(raw) = &from else {
        return Err(Flow::error("'from' must be raw or character"));
    };
    let mut typ = as_integer(&arg(&args, 1));
    // R's subtype: 1 for the legacy .lzma format, detected only here
    let mut alone = false;
    if typ == 5 {
        // R: comp_type_from_memory(p, len, TRUE, &subtype)
        let p = &raw.data;
        typ = if p.len() >= 2 && p[0] == 0x1f && p[1] == 0x8b {
            2
        } else if p.len() >= 2 && p[0] == 0x78 && p[1] == 0x9c {
            2
        } else if p.len() >= 10
            && &p[..3] == b"BZh"
            && (b'1'..=b'9').contains(&p[3])
            && (&p[4..10] == b"\x31\x41\x59\x26\x53\x59" || &p[4..10] == b"\x17\x72\x45\x38\x50\x90")
        {
            3
        } else if p.len() >= 5 && p[0] == 0xFD && &p[1..5] == b"7zXZ" {
            4
        } else if p.len() >= 5 && (p[..5] == [0xFF, b'L', b'Z', b'M', b'A'] || p[..5] == [b']', 0, 0, 0x80, 0]) {
            alone = true;
            4
        } else if p.len() >= 4 && p[..4] == [0x28, 0xB5, 0x2F, 0xFD] {
            6
        } else if p.len() >= 4 && p[0] == 0x89 && &p[1..4] == b"LZO" {
            // R: comp_type_from_memory — lzop is not supported by R either
            return Err(Flow::error("this is a lzop-compressed file which this build of R does not support"));
        } else {
            i.warning("unknown compression, assuming none")?;
            1
        };
    }
    match typ {
        1 => Ok(from),
        2 => {
            // zlib format or an in-memory gzip file; zlib/libdeflate stop
            // after the first stream or member, unlike the connection
            // decoders. This R build reports libdeflate's status: any
            // data error is LIBDEFLATE_BAD_DATA (1); a growable buffer
            // makes the insufficient-space status unreachable.
            let mut out = Vec::new();
            let ok = if raw.data.len() >= 2 && raw.data[0] == 0x1f && raw.data[1] == 0x8b {
                flate2::read::GzDecoder::new(&raw.data[..]).read_to_end(&mut out).is_ok()
            } else {
                flate2::read::ZlibDecoder::new(&raw.data[..]).read_to_end(&mut out).is_ok()
            };
            if !ok {
                return Err(Flow::error("internal error 1 in memDecompress(type = \"libdeflate\")"));
            }
            Ok(Value::raws(out))
        }
        3 => {
            // BZ2_bzBuffToBuffDecompress decodes one stream and stops;
            // the status codes are bzlib's
            let mut out = Vec::new();
            if let Err(e) = bzip2::read::BzDecoder::new(&raw.data[..]).read_to_end(&mut out) {
                let code = match e.get_ref().and_then(|x| x.downcast_ref::<bzip2::Error>()) {
                    Some(bzip2::Error::DataMagic) => -5,
                    Some(bzip2::Error::Data) => -4,
                    Some(bzip2::Error::Sequence) => -1,
                    Some(_) => -2,
                    // an i/o end-of-stream error is BZ_UNEXPECTED_EOF
                    None => -7,
                };
                return Err(Flow::error(format!("internal error {code} in memDecompress(type = \"bzip2\")")));
            }
            Ok(Value::raws(out))
        }
        4 => mem_decompress_xz(&raw.data, alone).map(Value::raws),
        6 => Err(Flow::error("robigo: Zstd decompression is not implemented yet")),
        _ => Ok(from),
    }
}

/// memDecompress(type = "xz"): one decoder call into a buffer of three
/// times the input, doubled and retried while the output is what runs
/// out. A failure that leaves no input unread is not an error (R returns
/// what was decoded).
// R: connections.c:do_memDecompress case 4
fn mem_decompress_xz(from: &[u8], alone: bool) -> RResult<Vec<u8>> {
    use liblzma::stream::{Action, Status};
    let mut outlen = 3 * from.len();
    loop {
        let mut strm = xz_decoder(alone)
            .map_err(|e| Flow::error(format!("cannot initialize lzma decoder, error {}", lzma_ret(e))))?;
        let mut buf = vec![0u8; outlen];
        let ret = strm.process(from, &mut buf, Action::Finish);
        let avail_in = from.len() - strm.total_in() as usize;
        if avail_in > 0 {
            match ret {
                // LZMA_OK or LZMA_BUF_ERROR: out of output buffer
                Ok(Status::Ok | Status::MemNeeded) => {
                    outlen *= 2;
                    continue;
                }
                _ => {
                    let code = match ret {
                        Ok(Status::StreamEnd) => 1,
                        Ok(_) => 4,
                        Err(e) => lzma_ret(e),
                    };
                    return Err(Flow::error(format!(
                        "internal error {code} in memDecompress(type = \"xz\") at {avail_in}"
                    )));
                }
            }
        }
        buf.truncate(strm.total_out() as usize);
        return Ok(buf);
    }
}

/// `gzcon(con, level, allowNonCompressed, text)` — connections.c:do_gzcon.
fn do_gzcon(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let icon = con_arg(&arg(&args, 0))?;
    let level = as_integer(&arg(&args, 1));
    if level == NA_INTEGER || !(0..=9).contains(&level) {
        return Err(Flow::error_call(call.clone(), "'level' must be one of 0 ... 9"));
    }
    let allow = as_logical(&arg(&args, 2));
    if allow == NA_LOGICAL {
        return Err(Flow::error_call(call.clone(), "'allowNonCompression' must be TRUE or FALSE"));
    }
    let text = as_logical(&arg(&args, 3));
    if text == NA_LOGICAL {
        return Err(Flow::error_call(call.clone(), "'text' must be TRUE or FALSE"));
    }
    let (imode, iclass, idesc, inner_open) = {
        let c = i.conns.get(icon)?;
        if c.class == "gzcon" {
            i.warning("this is already a 'gzcon' connection")?;
            return Ok(arg(&args, 0).clone());
        }
        (c.mode.clone(), c.class.clone(), c.description.clone(), c.isopen)
    };
    let mode = if imode == "r" || imode.starts_with("rb") {
        "rb"
    } else if imode == "w" || imode.starts_with("wb") {
        "wb"
    } else {
        return Err(Flow::error("can only use read- or write- binary connections"));
    };
    if iclass == "file" && (imode == "r" || imode == "w") {
        i.warning("using a text-mode 'file' connection may not work correctly")?;
    } else if iclass == "textConnection" && imode == "w" {
        return Err(Flow::error(
            "cannot create a 'gzcon' connection from a writable textConnection; maybe use rawConnection",
        ));
    }
    // R: `Connections[icon] = new` — the gzcon takes over the wrapped
    // connection's number, and the wrapped one leaves the table: the gzcon
    // owns it.
    let Some(wrapped) = i.conns.table[icon as usize].take() else { return Err(Flow::error("invalid connection")) };
    let kind = ConnKind::Gzcon {
        inner: Some(wrapped),
        level: level as u32,
        allow: allow == TRUE,
        data: Vec::new(),
        cur: 0,
        out: Vec::new(),
        writing: false,
    };
    let mut con = Connection::new("gzcon", &format!("gzcon({})", idesc), mode, kind);
    con.text = text == TRUE;
    con.canseek = false;
    let v = install_connection(i, icon as usize, con);
    if inner_open {
        let _ = gzcon_open(i, icon)?;
    }
    Ok(v)
}

/// R: connections.c:gzcon_open — validate the gzip header (or fall back to
/// reading the stream as-is with `allowNonCompression`), then inflate the
/// wrapped stream; or write the header for writing.
fn gzcon_open(i: &mut Interp, n: i32) -> RResult<bool> {
    let inner = lend_wrapped(i, n)?;
    let r = gzcon_open_lent(i, n, inner);
    restore_wrapped(i, n, inner);
    // A warning runs calling handlers (R code), so it waits until the
    // wrapped connection is back.
    match r? {
        Err(msg) => {
            i.warning(msg)?;
            Ok(false)
        }
        Ok(opened) => Ok(opened),
    }
}

/// The gzcon `n`'s wrapped connection, moved to a hidden slot (see
/// `ConnState::hide`) so the generic connection methods can work on it;
/// `restore_wrapped` takes it back. gzcon's own warnings wait until it
/// is back; only opening a wrapped connection that is not open yet can
/// warn (and run handlers) meanwhile — a handler that closes the hidden
/// slot by a made-up number leaves the gzcon with no wrapped connection,
/// whose next open or close is then an "invalid connection" error.
fn lend_wrapped(i: &mut Interp, n: i32) -> RResult<i32> {
    let ConnKind::Gzcon { inner, .. } = &mut i.conns.get(n)?.kind else { unreachable!() };
    let Some(w) = inner.take() else { return Err(Flow::error("invalid connection")) };
    Ok(i.conns.hide(w))
}

fn restore_wrapped(i: &mut Interp, n: i32, k: i32) {
    let w = i.conns.table[k as usize].take();
    if let Ok(ConnKind::Gzcon { inner, .. }) = i.conns.get(n).map(|c| &mut c.kind) {
        *inner = w;
    }
}

/// `Ok(Err(msg))`: the open failed with warning `msg`, for the caller.
fn gzcon_open_lent(i: &mut Interp, n: i32, inner: i32) -> RResult<Result<bool, &'static str>> {
    let mode = i.conns.get(n)?.mode.clone();
    let wasopen = i.conns.get(inner)?.isopen;
    if !wasopen && !open_in_mode(i, inner, &mode)? {
        return Ok(Ok(false));
    }
    let canwrite = i.conns.get(inner)?.canwrite;
    {
        let con = i.conns.get(n)?;
        con.isopen = true;
        con.canwrite = canwrite;
        con.canread = !canwrite;
        con.unread = None;
        con.save = -1000;
    }
    if !canwrite {
        // Reading: pull the wrapped stream and inflate it (as gzfile does).
        let raw = con_read_all(i, inner)?;
        let ConnKind::Gzcon { allow, .. } = i.conns.get(n)?.kind else { unreachable!() };
        // R: a refused stream leaves the gzcon open (isopen was set above)
        // with no inflate state, so every later read returns nothing.
        let mut refused = None;
        let data = if raw.len() < 2 || raw[0] != 0x1f || raw[1] != 0x8b {
            if !allow {
                refused = Some("file stream does not have gzip magic number");
                Vec::new()
            } else {
                raw
            }
        } else if raw.len() < 10 || raw[2] != 8 || raw[3] & 0xe0 != 0 {
            refused = Some("file stream does not have valid gzip header");
            Vec::new()
        } else {
            // R: gzcon_open skips the header byte by byte; a truncated
            // header just runs into the end of the stream.
            let flags = raw[3];
            let mut p = 10usize; // magic(2) method(1) flags(1) time(4) xfl(1) os(1)
            if flags & 0x04 != 0 {
                // EXTRA_FIELD
                p = match raw.get(p..p + 2) {
                    Some(b) => p + 2 + u16::from_le_bytes([b[0], b[1]]) as usize,
                    None => raw.len(),
                };
            }
            for mask in [0x08u8, 0x10] {
                // ORIG_NAME, COMMENT
                if flags & mask != 0 {
                    while p < raw.len() && raw[p] != 0 {
                        p += 1;
                    }
                    p += 1;
                }
            }
            if flags & 0x02 != 0 {
                p += 2; // HEAD_CRC
            }
            let body = &raw[p.min(raw.len())..];
            // R: connections.c:gzcon_read — inflate until the stream ends or
            // fails (what was inflated is returned either way); at the end,
            // the crc trailer is checked (a mismatch is only reported) and
            // the length is ignored.
            let mut dec = flate2::Decompress::new(false);
            let mut data = Vec::with_capacity(body.len() * 4);
            let ended = loop {
                if data.len() == data.capacity() {
                    data.reserve(data.len().max(4096));
                }
                let before = (dec.total_in(), dec.total_out());
                let at = (dec.total_in() as usize).min(body.len());
                match dec.decompress_vec(&body[at..], &mut data, flate2::FlushDecompress::None) {
                    Ok(flate2::Status::StreamEnd) => break true,
                    Ok(_) if (dec.total_in(), dec.total_out()) != before => {}
                    _ => break false,
                }
            };
            if ended {
                // gzcon_byte: EOF (-1) past the end of the wrapped stream.
                let t = (dec.total_in() as usize).min(body.len());
                let mut crc: u64 = 0;
                for k in t..t + 4 {
                    let b = body.get(k).map_or(u64::MAX, |&b| b as u64);
                    crc >>= 8;
                    crc = crc.wrapping_add(b << 24);
                }
                let sum = crc32(&data) as u64;
                if crc != sum {
                    i.write_err(&format!("crc error {:x} {:x}\n", crc, sum));
                }
            }
            data
        };
        let con = i.conns.get(n)?;
        let ConnKind::Gzcon { data: d, cur, .. } = &mut con.kind else { unreachable!() };
        *d = data;
        *cur = 0;
        if !wasopen {
            con_close_raw(i, inner)?;
        }
        if let Some(msg) = refused {
            return Ok(Err(msg));
        }
    } else {
        // Writing: the header is gzcon_open's; the body and trailer are
        // written at close (gzcon_close).
        con_write_bytes(i, inner, &[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3])?;
        let con = i.conns.get(n)?;
        let ConnKind::Gzcon { writing, .. } = &mut con.kind else { unreachable!() };
        *writing = true;
    }
    Ok(Ok(true))
}

/// crc32 of `data` (zlib's crc32(0, ...)), little trailer order.
fn crc32(data: &[u8]) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(data);
    c.sum()
}

/// R: connections.c:gzcon_close — finish the deflate stream, write the
/// crc and length trailer, and close the wrapped connection.
fn gzcon_close(i: &mut Interp, n: i32) -> RResult<()> {
    let inner = lend_wrapped(i, n)?;
    let r = gzcon_close_lent(i, n, inner);
    restore_wrapped(i, n, inner);
    r
}

fn gzcon_close_lent(i: &mut Interp, n: i32, inner: i32) -> RResult<()> {
    let canwrite = i.conns.get(n)?.canwrite;
    if canwrite {
        let (raw, level) = {
            let con = i.conns.get(n)?;
            let ConnKind::Gzcon { out, level, .. } = &mut con.kind else { unreachable!() };
            (std::mem::take(out), *level)
        };
        let crc = crc32(&raw);
        let total_in = raw.len() as u32;
        // deflateInit2(-MAX_WBITS) + deflate(Z_FINISH): the raw deflate
        // stream of everything buffered, in one go.
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(level));
        use std::io::Write;
        enc.write_all(&raw)
            .and_then(|_| enc.finish())
            .map_err(|_| Flow::error("writing error whilst flushing 'gzcon' connection"))
            .and_then(|body| {
                // R: putLong of the trailer only after the body was
                // written (a failed write errors out first).
                con_write_bytes(i, inner, &body)?;
                let mut trailer = Vec::with_capacity(8);
                trailer.extend_from_slice(&crc.to_le_bytes());
                trailer.extend_from_slice(&total_in.to_le_bytes());
                con_write_bytes(i, inner, &trailer)
            })?;
    }
    if i.conns.get(inner)?.isopen {
        con_close_raw(i, inner)?;
    }
    i.conns.get(n)?.isopen = false;
    Ok(())
}

fn do_gzfile(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let fname = match call {
        Value::Lang(l) => l.fun_sym().map(|s| s.name().to_string()).unwrap_or_default(),
        _ => String::new(),
    };
    let mut ty = match fname.as_str() {
        "bzfile" => 1,
        "xzfile" => 2,
        "zstdfile" => 3,
        _ => 0,
    };
    let file = match arg(&args, 0) {
        Value::Str(s) if s.data.len() == 1 && s.data[0].is_some() => s.data[0].clone().unwrap().to_string(),
        _ => return Err(Flow::error("invalid 'description' argument")),
    };
    let open = match arg(&args, 1) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error("invalid 'open' argument")),
    };
    let enc = match arg(&args, 2) {
        Value::Str(s) if s.data.len() == 1 && s.data[0].as_ref().is_some_and(|e| e.len() <= 100) => {
            s.data[0].clone().unwrap().to_string()
        }
        _ => return Err(Flow::error("invalid 'encoding' argument")),
    };
    let compress = as_integer(&arg(&args, 3));
    let limit = match ty {
        3 => 19,
        _ => 9,
    };
    if compress == NA_INTEGER || (ty < 2 && compress < 0) || compress.abs() > limit {
        return Err(Flow::error("invalid 'compress' argument"));
    }
    let mut alone = false;
    if ty == 0 && (open.is_empty() || open.starts_with('r')) {
        // check magic no
        // R: connections.c:comp_type_from_file(name, FALSE, &subtype)
        match compressed_file_type(&expand_file_name(&file)) {
            None | Some(Compression::Gzip) => {}
            Some(Compression::Bzip2) => ty = 1,
            Some(Compression::Xz { alone: a }) => {
                ty = 2;
                alone = a;
            }
            Some(Compression::Zstd) => ty = 3,
            Some(Compression::Lzop) => {
                return Err(Flow::error("this is a lzop-compressed file which this build of R does not support"));
            }
        }
    }
    let codec = match ty {
        0 => Codec::Gzip,
        1 => Codec::Bzip2,
        2 => Codec::Xz { alone },
        _ => return Err(Flow::error("robigo: zstd compressed file connections are not implemented yet")),
    };
    let ncon = next_connection(i)?;
    let mode = if open.is_empty() { "rb".to_string() } else { open.clone() };
    let kind = ConnKind::Gz {
        codec,
        level: compress,
        data: Vec::new(),
        cur: 0,
        out: Vec::new(),
        writing: false,
        append: false,
        xz: None,
    };
    let mut con = Connection::new(codec.class(), &file, &mode, kind);
    con.blocking = true;
    con.canseek = false;
    con.encname = enc;
    let v = install_connection(i, ncon, con);
    if !open.is_empty() {
        checked_open(i, ncon as i32)?;
    }
    Ok(v)
}

/// `.Internal(textConnection(name, object, open, env, type))`
fn do_textconnection(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let desc = match arg(&args, 0) {
        Value::Str(s) if s.data.len() == 1 && s.data[0].is_some() => s.data[0].clone().unwrap().to_string(),
        _ => return Err(Flow::error("invalid 'description' argument")),
    };
    let stext = arg(&args, 1);
    let open = match arg(&args, 2) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error("invalid 'open' argument")),
    };
    let venv = arg(&args, 3);
    if venv.is_null() {
        return Err(Flow::error("use of NULL environment is defunct"));
    }
    let Value::Env(venv) = venv else { return Err(Flow::error("invalid 'environment' argument")) };
    let typ = as_integer(&arg(&args, 4));
    if typ == NA_INTEGER {
        return Err(Flow::error("invalid 'encoding' argument"));
    }
    let ncon = next_connection(i)?;
    let mut con = if open.is_empty() || open.starts_with('r') {
        let Value::Str(t) = &stext else { return Err(Flow::error("invalid 'text' argument")) };
        // R: connections.c:text_init
        let mut data = Vec::new();
        for l in &t.data {
            data.extend_from_slice(l.as_deref().unwrap_or("NA").as_bytes());
            data.push(b'\n');
        }
        let mut c = Connection::new("textConnection", &desc, "r", ConnKind::TextIn { data, cur: 0 });
        c.isopen = true;
        c.canwrite = false;
        c
    } else if open.starts_with('w') || open.starts_with('a') {
        // R: connections.c:newouttext / outtext_init
        let (description, namesym) = match &stext {
            Value::Null => ("NULL".to_string(), None),
            Value::Str(t) if t.data.len() == 1 => {
                let d = t.data[0].as_deref().unwrap_or("NA").to_string();
                let sym = Sym::new(&d);
                (d, Some(sym))
            }
            _ => return Err(Flow::error("invalid 'text' argument")),
        };
        let mut data: Vec<RStr> = Vec::new();
        if let Some(sym) = namesym {
            if open.starts_with('w') {
                if venv.has_local(sym) {
                    venv.set_binding_locked(sym, false);
                }
                venv.set_local(sym, Value::strs(vec![]));
            } else {
                match venv.lookup(sym) {
                    Some((Value::Str(v), _)) => data = v.data.clone(),
                    _ => {
                        i.warning("text connection: appending to a non-existent char vector")?;
                        venv.set_local(sym, Value::strs(vec![]));
                    }
                }
                venv.set_binding_locked(sym, true);
            }
        }
        let mut c = Connection::new(
            "textConnection",
            &description,
            &open,
            ConnKind::TextOut { namesym, env: venv.clone(), data, lastline: Vec::new() },
        );
        c.isopen = true;
        c.canread = false;
        c
    } else {
        return Err(Flow::error("unsupported mode"));
    };
    con.blocking = false;
    Ok(install_connection(i, ncon, con))
}

fn do_textconvalue(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let v = arg(&args, 0);
    if !v.inherits("textConnection") {
        return Err(Flow::error("'con' is not a textConnection"));
    }
    let con = i.conns.get(as_integer(&v))?;
    if !con.canwrite {
        return Err(Flow::error("'con' is not an output textConnection"));
    }
    match &con.kind {
        ConnKind::TextOut { data, .. } => Ok(Value::strs(data.clone())),
        _ => Err(Flow::error("'con' is not an output textConnection")),
    }
}

fn do_open(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = con_arg(&arg(&args, 0))?;
    i.conns.get(n)?;
    if n < 3 {
        return Err(Flow::error("cannot open standard connections"));
    }
    if i.conns.get(n)?.isopen {
        i.warning("connection is already open")?;
        return Ok(Value::Null);
    }
    let open = str1_arg(&arg(&args, 1), "open")?;
    let block = as_logical(&arg(&args, 2));
    if block == NA_LOGICAL {
        return Err(Flow::error_call(call.clone(), "NA in coercion to boolean"));
    }
    let con = i.conns.get(n)?;
    if !open.is_empty() {
        con.mode = open.chars().take(4).collect();
    }
    con.blocking = block != 0;
    if !con_open(i, n)? {
        return Err(Flow::error("cannot open the connection"));
    }
    if let ConnKind::Sock(sock) = &mut i.conns.get(n)?.kind {
        sock.release_server();
    }
    Ok(Value::Null)
}

fn do_isopen(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let con = i.conns.get(as_integer(&arg(&args, 0)))?;
    let rw = as_integer(&arg(&args, 1));
    let mut res = con.isopen;
    match rw {
        0 => {}
        1 => res &= con.canread,
        2 => res &= con.canwrite,
        _ => return Err(Flow::error("unknown 'rw' value")),
    }
    Ok(Value::lgl(res))
}

fn do_isincomplete(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = con_arg(&arg(&args, 0))?;
    Ok(Value::lgl(i.conns.get(n)?.incomplete))
}

fn do_isseekable(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = con_arg(&arg(&args, 0))?;
    Ok(Value::lgl(i.conns.get(n)?.canseek))
}

fn do_close(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = con_arg(&arg(&args, 0))?;
    if n < 3 {
        return Err(Flow::error("cannot close standard connections"));
    }
    for j in 0..i.conns.sink_number {
        if n == i.conns.sink_cons[j] {
            return Err(Flow::error("cannot close 'output' sink connection"));
        }
    }
    if n == i.conns.error_con {
        return Err(Flow::error("cannot close 'message' sink connection"));
    }
    i.conns.get(n)?;
    let st = con_destroy(i, n)?;
    Ok(if st != NA_INTEGER { Value::int(st) } else { Value::Null })
}

fn do_flush(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = con_arg(&arg(&args, 0))?;
    if i.conns.get(n)?.canwrite {
        conn_flush(i, n)?;
    }
    Ok(Value::Null)
}

/// `readLines(con, n, ok, warn, encoding, skipNul)`
fn do_readlines(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let ncon = con_arg(&arg(&args, 0))?;
    i.conns.get(ncon)?;
    let n = crate::coerce::as_vec_size(i, &arg(&args, 1))?;
    if n == -999 {
        return Err(Flow::error("invalid 'n' argument"));
    }
    let ok = as_logical(&arg(&args, 2));
    if ok == NA_LOGICAL {
        return Err(Flow::error("invalid 'ok' argument"));
    }
    let warn = as_logical(&arg(&args, 3));
    if warn == NA_LOGICAL {
        return Err(Flow::error("invalid 'warn' argument"));
    }
    let encoding = match arg(&args, 4) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].clone(),
        _ => return Err(Flow::error("invalid 'encoding' value")),
    };
    let skip_nul = as_logical(&arg(&args, 5));
    if skip_nul == NA_LOGICAL {
        return Err(Flow::error("invalid 'skipNul' argument"));
    }
    let wasopen = i.conns.get(ncon)?.isopen;
    if !wasopen {
        let con = i.conns.get(ncon)?;
        con.utf8out = true; // a request
        let mode = std::mem::replace(&mut con.mode, "rt".into());
        let opened = con_open(i, ncon)?;
        i.conns.get(ncon)?.mode = mode;
        if !opened {
            return Err(Flow::error("cannot open the connection"));
        }
        if !i.conns.get(ncon)?.canread {
            check_close(i, ncon)?;
            return Err(Flow::error("cannot read from this connection"));
        }
    } else if !i.conns.get(ncon)?.canread {
        return Err(Flow::error("cannot read from this connection"));
    } else {
        // R: for a non-blocking connection, more input may have become
        // available, so re-position (tell then seek the read position).
        let (canseek, blocking) = {
            let c = i.conns.get(ncon)?;
            (c.canseek, c.blocking)
        };
        if canseek && !blocking {
            let tell = con_seek(i, ncon, f64::NAN, 1, 1)?;
            con_seek(i, ncon, tell, 1, 1)?;
        }
    }
    // R: connections.c:do_readLines
    let oenc = match encoding.as_deref() {
        _ if i.conns.get(ncon)?.utf8out => Encoding::Utf8,
        Some("UTF-8") => Encoding::Utf8,
        Some("latin1") => Encoding::Latin1,
        Some("bytes") => Encoding::Bytes,
        _ => Encoding::Native,
    };
    let nnn = if n < 0 { i64::MAX } else { n };
    let mut ans: Vec<RStr> = Vec::new();
    let mut nul_warnings: Vec<usize> = Vec::new();
    // R: the incomplete final line is stored like any other before
    // `no_more_lines` handling; `incomplete_buf` holds it for the
    // pushback-or-warn decision.
    let mut incomplete_buf: Option<Vec<u8>> = None;
    i.conns.get(ncon)?.incomplete = false;
    let lines = ReadLines { nnn, skip_nul: skip_nul != 0, warn: warn != 0, oenc };
    let eof = if i.conns.get(ncon)?.reads_need_interp() {
        // an error ends the input, then raises (a native connection's, or
        // dummy_fgetc's warning under `options(warn = 2)`)
        let mut err = None;
        let getc = || {
            rconn_fgetc(i, ncon).unwrap_or_else(|e| {
                err = Some(e);
                EOF
            })
        };
        let eof = lines.read(getc, &mut ans, &mut nul_warnings, &mut incomplete_buf);
        if let Some(e) = err {
            // R warns inline, so the warnings of the lines before the
            // error stand; the line the error cut short never got its own
            for k in nul_warnings.into_iter().filter(|&k| k < ans.len()) {
                i.warning(format!("line {} appears to contain an embedded nul", k))?;
            }
            return Err(e);
        }
        eof
    } else {
        let con = i.conns.get(ncon)?;
        if con.is_contiguous_buffered() {
            lines.read_buffered(con, &mut ans, &mut nul_warnings, &mut incomplete_buf)
        } else {
            lines.read(|| rconn_fgetc_builtin(con), &mut ans, &mut nul_warnings, &mut incomplete_buf)
        }
    };
    for k in nul_warnings {
        i.warning(format!("line {} appears to contain an embedded nul", k))?;
    }
    if !wasopen {
        con_close_raw(i, ncon)?;
    }
    if eof {
        match incomplete_buf {
            Some(buf) => {
                let con = i.conns.get(ncon)?;
                if con.text && !con.blocking && con.class != "gzfile" {
                    // R: the pushed-back line is not part of the result.
                    ans.pop();
                    con.pushback.push(buf);
                    con.pos_pushback = 0;
                    con.incomplete = true;
                } else {
                    let desc = con.description.clone();
                    if warn != 0 {
                        i.warning(format!("incomplete final line found on '{}'", desc))?;
                    }
                }
            }
            // EOF right after a newline: R stores an empty string in the
            // slot and then truncates the result to exclude it.
            None => {
                ans.pop();
            }
        }
    }
    if (ans.len() as i64) < nnn && ok == 0 {
        return Err(Flow::error("too few lines read in readLines"));
    }
    Ok(Value::strs(ans))
}

/// readLines' line loop, over a connection's characters.
struct ReadLines {
    nnn: i64,
    skip_nul: bool,
    warn: bool,
    /// The lines' encoding mark: their bytes are kept as read
    /// (R: `mkCharCE(qbuf, oenc)`).
    oenc: Encoding,
}

impl ReadLines {
    /// Read up to `nnn` lines into `ans`; whether input ended.
    #[inline(always)]
    fn read(
        &self,
        mut getc: impl FnMut() -> i32,
        ans: &mut Vec<RStr>,
        nul_warnings: &mut Vec<usize>,
        incomplete_buf: &mut Option<Vec<u8>>,
    ) -> bool {
        let mut eof = false;
        let mut nread: i64 = 0;
        let mut buf: Vec<u8> = Vec::new();
        while nread < self.nnn && !eof {
            buf.clear();
            let mut c;
            loop {
                c = getc();
                if c == EOF {
                    break;
                }
                if self.skip_nul && c == 0 {
                    continue;
                }
                if c == '\n' as i32 {
                    break;
                }
                buf.push(c as u8);
            }
            // The string stops at the first NUL (mkCharCE semantics).
            let nul = buf.contains(&0);
            let mut line: &[u8] = &buf;
            if nul {
                let end = line.iter().position(|&b| b == 0).unwrap_or(line.len());
                line = &line[..end];
            }
            if nread == 0 && line.starts_with(b"\xef\xbb\xbf") {
                line = &line[3..];
            }
            ans.push(Some(RString::from_bytes(line, self.oenc)));
            if nul && self.warn {
                nul_warnings.push(nread as usize + 1);
            }
            nread += 1;
            if c == EOF {
                eof = true;
                if !buf.is_empty() {
                    *incomplete_buf = Some(std::mem::take(&mut buf));
                }
            }
        }
        eof
    }

    #[inline(never)]
    fn read_buffered(
        &self,
        con: &mut Connection,
        ans: &mut Vec<RStr>,
        nul_warnings: &mut Vec<usize>,
        incomplete_buf: &mut Option<Vec<u8>>,
    ) -> bool {
        let mut eof = false;
        let mut nread: i64 = 0;
        let mut buf: Vec<u8> = Vec::new();
        while nread < self.nnn && !eof {
            buf.clear();
            if con.save != -1000 {
                let s = con.save;
                con.save = -1000;
                if s == '\n' as i32 {
                    ans.push(rstr(""));
                    nread += 1;
                    continue;
                } else if s != EOF {
                    buf.push(s as u8);
                }
            }
            let mut found_nl = false;
            while !found_nl {
                let Some(slice) = con.get_available_bytes() else {
                    eof = true;
                    break;
                };
                if let Some(pos) = slice.iter().position(|&b| b == b'\n' || b == b'\r') {
                    let d = slice[pos];
                    buf.extend_from_slice(&slice[..pos]);
                    con.consume_bytes(pos + 1);
                    if d == b'\r' {
                        if let Some(next_slice) = con.get_available_bytes() {
                            if !next_slice.is_empty() {
                                if next_slice[0] == b'\n' {
                                    con.consume_bytes(1);
                                } else if next_slice[0] == b'\r' {
                                    con.consume_bytes(1);
                                    con.save = '\n' as i32;
                                }
                            }
                        }
                    }
                    found_nl = true;
                } else {
                    buf.extend_from_slice(slice);
                    let len = slice.len();
                    con.consume_bytes(len);
                }
            }
            if eof && buf.is_empty() {
                ans.push(rstr(""));
                break;
            }
            let nul = buf.contains(&0);
            let mut line: &[u8] = &buf;
            let mut filtered = Vec::new();
            if nul {
                if self.skip_nul {
                    filtered.extend(line.iter().copied().filter(|&b| b != 0));
                    line = &filtered;
                } else {
                    let end = line.iter().position(|&b| b == 0).unwrap_or(line.len());
                    line = &line[..end];
                }
            }
            if nread == 0 && line.starts_with(b"\xef\xbb\xbf") {
                line = &line[3..];
            }
            ans.push(Some(RString::from_bytes(line, self.oenc)));
            if nul && self.warn && !self.skip_nul {
                nul_warnings.push(nread as usize + 1);
            }
            nread += 1;
            if eof && !buf.is_empty() {
                *incomplete_buf = Some(std::mem::take(&mut buf));
            }
        }
        eof
    }
}

/// `writeLines(text, con, sep, useBytes)`
fn do_writelines(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let a0 = arg(&args, 0);
    let Value::Str(t) = &a0 else {
        return Err(Flow::error("invalid 'text' argument"));
    };
    let con_num = con_arg(&arg(&args, 1))?;
    i.conns.get(con_num)?;
    let use_bytes = as_logical(&arg(&args, 3));
    // Outside a UTF-8 locale the strings are written translated
    // (translateChar0) or, with useBytes, as their bytes; in one, so are
    // the strings whose bytes are not their UTF-8 text.
    let translate = !crate::locale::utf8locale();
    let sep = match arg(&args, 2) {
        Value::Str(s) => match s.data.first().cloned().flatten() {
            Some(s) if (translate || s.has_foreign_bytes()) && use_bytes != NA_LOGICAL => {
                writelines_native(&s, use_bytes != 0)?.into_owned()
            }
            Some(s) => s.to_string(),
            None => "NA".into(),
        },
        _ => return Err(Flow::error("invalid 'sep' argument")),
    };
    if use_bytes == NA_LOGICAL {
        return Err(Flow::error("invalid 'useBytes' argument"));
    }
    let wasopen = i.conns.get(con_num)?.isopen;
    if !wasopen {
        let con = i.conns.get(con_num)?;
        let mode = std::mem::replace(&mut con.mode, "wt".into());
        let ok = con_open(i, con_num)?;
        i.conns.get(con_num)?.mode = mode;
        if !ok {
            return Err(Flow::error("cannot open the connection"));
        }
    }
    if !i.conns.get(con_num)?.canwrite {
        if !wasopen {
            check_close(i, con_num)?;
        }
        return Err(Flow::error("cannot write to this connection"));
    }
    let write_chunk = |i: &mut Interp, con_num: i32, chunk: &str| -> RResult<()> {
        if con_num == i.conns.output_con {
            let mut j = 0;
            let mut cur = con_num;
            loop {
                conn_write(i, cur, chunk)?;
                conn_flush(i, cur)?;
                cur = i.conns.active_sink(j);
                j += 1;
                if cur <= 0 {
                    break;
                }
            }
            Ok(())
        } else {
            conn_write(i, con_num, chunk)
        }
    };
    let mut out = String::with_capacity(65536);
    for s in &t.data {
        let piece = match s {
            Some(s) if translate || s.has_foreign_bytes() => writelines_native(s, use_bytes != 0)?,
            Some(s) => Cow::Borrowed(&**s),
            None => Cow::Borrowed("NA"),
        };
        if out.len() + piece.len() + sep.len() > 65536 && !out.is_empty() {
            write_chunk(i, con_num, &out)?;
            out.clear();
        }
        out.push_str(&piece);
        out.push_str(&sep);
    }
    if !out.is_empty() {
        write_chunk(i, con_num, &out)?;
    }
    if !wasopen {
        check_close(i, con_num)?;
    }
    Ok(Value::Null)
}

/// One string of writeLines() outside a UTF-8 locale, or with bytes that
/// are not its text: its bytes for `useBytes = TRUE` and "bytes" strings,
/// otherwise translated to the native encoding.
// R: connections.c:do_writelines (translateChar0)
#[cold]
#[inline(never)]
fn writelines_native(s: &RString, use_bytes: bool) -> RResult<Cow<'_, str>> {
    if s.is_ascii() {
        return Ok(Cow::Borrowed(s));
    }
    let bytes = if use_bytes || s.is_bytes() { s.raw_bytes() } else { crate::locale::translate_char(s)? };
    Ok(Cow::Owned(crate::locale::native_to_display(&bytes)))
}

/// R: scan.c:do_readln (non-interactive: simulate a CR response).
// R: scan.c:do_readln
fn do_readln(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let prompt = arg(&args, 0);
    if prompt.is_null() {
        crate::console::set_prompt(""); // precaution
    } else {
        let pv = i.coerce(&prompt, VType::Str)?;
        if let Value::Str(s) = &pv {
            if !s.data.is_empty() {
                crate::console::set_prompt(s.data[0].as_deref().unwrap_or("NA"));
            }
        }
    }
    if i.interactive {
        // R: MAXELTSIZE
        const MAXELTSIZE: usize = 8192;
        let mut buf: Vec<u8> = Vec::new();
        // skip space or tab
        let mut c = crate::console::prompt_getchar();
        while c == ' ' as i32 || c == '\t' as i32 {
            c = crate::console::prompt_getchar();
        }
        if c != '\n' as i32 && c != EOF {
            buf.push(c as u8);
            loop {
                c = crate::console::prompt_getchar();
                if c == '\n' as i32 || c == EOF {
                    break;
                }
                if buf.len() < MAXELTSIZE - 2 {
                    buf.push(c as u8);
                }
            }
        }
        // now strip white space off the end as well
        while buf.last().is_some_and(|&b| b == b' ' || b == b'\t') {
            buf.pop();
        }
        crate::console::set_prompt("");
        Ok(Value::str(&String::from_utf8_lossy(&buf)))
    } else {
        // simulate CR as response
        i.write_out(&format!("{}\n", crate::console::prompt()));
        Ok(Value::str(""))
    }
}

fn do_sink(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let icon = as_integer(&arg(&args, 0));
    let close_on_exit = as_logical(&arg(&args, 1));
    if close_on_exit == NA_LOGICAL {
        return Err(Flow::error("invalid 'closeOnExit' argument"));
    }
    let errcon = as_logical(&arg(&args, 2));
    if errcon == NA_LOGICAL {
        return Err(Flow::error("invalid 'type' argument"));
    }
    let tee = as_logical(&arg(&args, 3));
    if tee == NA_LOGICAL {
        return Err(Flow::error("invalid 'split' argument"));
    }
    if errcon == 0 {
        // allow space for cat() to use sink()
        if icon >= 0 && i.conns.sink_number >= NSINKS - 2 {
            return Err(Flow::error("sink stack is full"));
        }
        switch_or_tee_stdout(i, icon, close_on_exit != 0, tee != 0)?;
    } else if icon < 0 || icon == 2 {
        i.conns.error_con = 2;
    } else {
        i.conns.get(icon)?;
        i.conns.error_con = icon;
    }
    Ok(Value::Null)
}

fn do_sinknumber(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let errcon = as_logical(&arg(&args, 0));
    if errcon == NA_LOGICAL {
        return Err(Flow::error("invalid 'type' argument"));
    }
    Ok(Value::int(if errcon != 0 { i.conns.sink_number as i32 } else { i.conns.error_con }))
}

fn do_sumconnection(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let con = i.conns.get(as_integer(&arg(&args, 0)))?;
    Ok(Value::named_list(vec![
        ("description", Value::str(&con.description)),
        ("class", Value::str(&con.class)),
        ("mode", Value::str(&con.mode)),
        ("text", Value::str(if con.text { "text" } else { "binary" })),
        ("opened", Value::str(if con.isopen { "opened" } else { "closed" })),
        ("can read", Value::str(if con.canread { "yes" } else { "no" })),
        ("can write", Value::str(if con.canwrite { "yes" } else { "no" })),
    ]))
}

fn do_getconnection(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let what = as_integer(&arg(&args, 0));
    if what == NA_INTEGER {
        return Err(Flow::error("there is no connection NA"));
    }
    if what < 0 || what as usize >= NCONNECTIONS || i.conns.table[what as usize].is_none() {
        return Err(Flow::error(format!("there is no connection {}", what)));
    }
    let con = i.conns.get(what)?;
    Ok(connection_value(con, what as usize))
}

fn do_getallconnections(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::ints((0..NCONNECTIONS).filter(|&k| i.conns.table[k].is_some()).map(|k| k as i32).collect()))
}

fn do_pushback(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let stext = match arg(&args, 0) {
        Value::Str(s) => s.data.clone(),
        _ => return Err(Flow::error("invalid 'data' argument")),
    };
    let con = i.conns.get(as_integer(&arg(&args, 1)))?;
    let newline = as_logical(&arg(&args, 2));
    if newline == NA_LOGICAL {
        return Err(Flow::error("invalid 'newLine' argument"));
    }
    if !con.canread && !con.isopen {
        return Err(Flow::error("can only push back on open readable connections"));
    }
    if !con.text {
        return Err(Flow::error("can only push back on text-mode connections"));
    }
    for t in stext.iter().rev() {
        let mut b = t.as_deref().unwrap_or("NA").as_bytes().to_vec();
        if newline != 0 {
            b.push(b'\n');
        }
        if !b.is_empty() {
            con.pushback.push(b);
        }
    }
    con.pos_pushback = 0;
    Ok(Value::Null)
}

fn do_pushbacklength(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let con = i.conns.get(as_integer(&arg(&args, 0)))?;
    Ok(Value::int(con.pushback.len() as i32))
}

fn do_clearpushback(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let con = i.conns.get(as_integer(&arg(&args, 0)))?;
    con.pushback.clear();
    con.pos_pushback = 0;
    Ok(Value::Null)
}

// ----------------------------------------------------------------------
// raw connections

/// `.Internal(rawConnection(deparse(substitute(object)), object, open))`
// R: connections.c do_rawconnection
fn do_rawconnection(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let desc = match arg(&args, 0) {
        Value::Str(s) if s.data.len() == 1 && s.data[0].is_some() => s.data[0].clone().unwrap().to_string(),
        _ => return Err(Flow::error("invalid 'description' argument")),
    };
    let raw = match arg(&args, 1) {
        Value::Raw(r) => r.data.clone(),
        _ => return Err(Flow::error("invalid 'raw' argument")),
    };
    let open = match arg(&args, 2) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error("invalid 'open' argument")),
    };
    if open.contains('t') {
        return Err(Flow::error("invalid 'open' argument"));
    }
    let ncon = next_connection(i)?;
    // R: connections.c newraw
    let nbytes = raw.len();
    let mut con = Connection::new("rawConnection", &desc, &open, ConnKind::Raw { data: raw, pos: 0, nbytes });
    con.isopen = true;
    con.text = false;
    con.blocking = true;
    con.canseek = true;
    con.canwrite = open.starts_with('w') || open.starts_with('a');
    con.canread = open.starts_with('r');
    if open.len() >= 2 && open.as_bytes()[1] == b'+' {
        con.canread = true;
        con.canwrite = true;
    }
    let v = install_connection(i, ncon, con);
    if open.starts_with('a') {
        con_seek(i, ncon as i32, 0.0, 3, 0)?;
    }
    Ok(v)
}

/// `.Internal(rawConnectionValue(con))`
// R: connections.c do_rawconvalue
fn do_rawconvalue(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let v = arg(&args, 0);
    if !v.inherits("rawConnection") {
        return Err(Flow::error("'con' is not a rawConnection"));
    }
    let n = as_integer(&v);
    if i.conns.get(n)?.class != "rawConnection" {
        return Err(Flow::error("internal connection is not a rawConnection"));
    }
    let con = i.conns.get(n)?;
    if !con.canwrite {
        return Err(Flow::error("'con' is not an output rawConnection"));
    }
    match &con.kind {
        ConnKind::Raw { data, nbytes, .. } => Ok(Value::raws(data[..*nbytes].to_vec())),
        _ => Err(Flow::error("'con' is not an output rawConnection")),
    }
}

// ----------------------------------------------------------------------
// seeking

/// `Rconn_seek` on connection `n`: returns the old position.
// R: connections.c do_seek, file_seek, raw_seek, buff_seek
fn con_seek(i: &mut Interp, n: i32, pos_arg: f64, origin: i32, rw: i32) -> RResult<f64> {
    let mut r#where = pos_arg;
    let con = i.conns.get(n)?;
    if let ConnKind::File { buffered: true, .. } = con.kind {
        return buff_seek(i, n, r#where, origin, rw);
    }
    if rw != 2 && con.unread.is_some() && con.canseek {
        // R: connections.c:buff_seek, for the bytes `unread` gave back
        let left = con.unread.as_ref().map_or(0, |u| u.bytes.len() - u.pos);
        if r#where.is_nan() {
            let tell = con_seek_raw(i, n, r#where, origin, rw)?;
            let conv = i.conns.get(n)?.inconv.is_some();
            return Ok(if conv { tell } else { tell - left as f64 });
        }
        if origin == 2 {
            if (0.0..left as f64).contains(&r#where) {
                if let Some(u) = con.unread.as_mut() {
                    u.pos += r#where as usize;
                }
                return con_seek_raw(i, n, f64::NAN, origin, rw);
            }
            r#where -= left as f64;
        }
        con.drop_unread();
    }
    con_seek_raw(i, n, r#where, origin, rw)
}

// R: connections.c buff_seek, for a file's con->buff (`rbuf`) and the
// bytes `unread` gave back, which R would still hold there
fn buff_seek(i: &mut Interp, n: i32, mut r#where: f64, origin: i32, rw: i32) -> RResult<f64> {
    if rw == 2 {
        return con_seek_raw(i, n, r#where, origin, rw);
    }
    let con = i.conns.get(n)?;
    let unread = con.unread.as_ref().map_or(0, |u| u.bytes.len() - u.pos);
    let ConnKind::File { rbuf, rpos, .. } = &mut con.kind else { unreachable!("buff_seek on another kind") };
    let held = rbuf.len() - *rpos;
    if r#where.is_nan() {
        let tell = con_seek_raw(i, n, r#where, origin, rw)?;
        // a re-encoding connection's own buffer is not counted (it read
        // its input as R's iconv does)
        let conv = i.conns.get(n)?.inconv.is_some();
        return Ok(tell - if conv { held } else { held + unread } as f64);
    }
    // R: `con->buff_pos += (size_t) where` for any `where < unread_len`: the
    // cast truncates, and wraps a negative offset into a step back. A step
    // back past what is held seeks the file to the same place instead.
    if origin == 2 && r#where < (held + unread) as f64 && buff_step(con, r#where as i64) {
        return con_seek_raw(i, n, f64::NAN, origin, rw);
    }
    if origin == 2 {
        r#where -= (held + unread) as f64;
    }
    con.drop_unread();
    if let ConnKind::File { rbuf, rpos, .. } = &mut con.kind {
        rbuf.clear();
        *rpos = 0;
    }
    con_seek_raw(i, n, r#where, origin, rw)
}

/// Move the read position of a buffered file connection `step` bytes
/// within what it holds: the `unread` bytes first, then `rbuf`. A step back
/// past what is still held (before the `unread` bytes, or the start of
/// `rbuf`, where GNU R reads outside its buffer) is not taken: `false`.
fn buff_step(con: &mut Connection, step: i64) -> bool {
    let mut skip = step.unsigned_abs() as usize;
    if let Some(u) = con.unread.as_mut() {
        if step < 0 {
            let Some(p) = u.pos.checked_sub(skip) else { return false };
            u.pos = p;
            return true;
        }
        let k = skip.min(u.bytes.len() - u.pos);
        u.pos += k;
        skip -= k;
        if u.pos == u.bytes.len() {
            con.drop_unread();
        }
    }
    let ConnKind::File { rpos, .. } = &mut con.kind else { return false };
    *rpos = if step < 0 {
        let Some(p) = rpos.checked_sub(skip) else { return false };
        p
    } else {
        *rpos + skip
    };
    true
}

/// `con->seek(con, where, origin, rw)` of connection `n`'s kind.
fn con_seek_raw(i: &mut Interp, n: i32, r#where: f64, origin: i32, rw: i32) -> RResult<f64> {
    let con = i.conns.get(n)?;
    match con.kind {
        // R: connections.c null_seek — fifos, url-libcurl and gzcon cannot seek.
        ConnKind::File { fifo: true, .. } | ConnKind::Url { .. } | ConnKind::Gzcon { .. } => {
            Err(Flow::error("'seek' not enabled for this connection"))
        }
        ConnKind::File { fifo: false, .. } => file_seek(con, r#where, origin, rw),
        ConnKind::Raw { .. } => raw_seek(con, r#where, origin),
        ConnKind::Native { .. } => native_bridge()?.conn_seek(i, n, r#where, origin, rw),
        ConnKind::Terminal
        | ConnKind::TextIn { .. }
        | ConnKind::TextOut { .. }
        | ConnKind::Gz { .. }
        | ConnKind::Sock(_)
        | ConnKind::ServSock { .. } => Err(Flow::error("seek is not relevant for text connection")),
    }
}

// R: connections.c file_seek (the read buffer holds bytes already
// consumed from the OS file)
fn file_seek(con: &mut Connection, r#where: f64, origin: i32, rw: i32) -> RResult<f64> {
    use std::io::Seek;
    let (canread, canwrite) = (con.canread, con.canwrite);
    let ConnKind::File { fp: Some(f), rbuf, rpos: rb, wbuf, last_was_write, frpos, wpos, buffered, .. } = &mut con.kind
    else {
        return Ok(0.0);
    };
    let _ = flush_file_wbuf(f, wbuf);
    // make sure both positions are set (R's con->buff is past f_tell;
    // stdio's buffer is not)
    let os_pos = f.stream_position().unwrap_or(0);
    let stdio_held = if *buffered { 0 } else { (rbuf.len() - *rb) as u64 };
    if *last_was_write {
        *wpos = os_pos;
    } else {
        *frpos = os_pos - stdio_held;
    }
    let mut pos = if *last_was_write { *wpos } else { *frpos };
    if rw == 1 {
        if !canread {
            return Err(Flow::error("connection is not open for reading"));
        }
        pos = *frpos;
        *last_was_write = false;
    }
    if rw == 2 {
        if !canwrite {
            return Err(Flow::error("connection is not open for writing"));
        }
        pos = *wpos;
        *last_was_write = true;
    }
    if r#where.is_nan() {
        return Ok(pos as f64);
    }
    let whence = match origin {
        2 => std::io::SeekFrom::Current(r#where as i64),
        3 => std::io::SeekFrom::End(r#where as i64),
        _ => std::io::SeekFrom::Start(r#where as u64),
    };
    if !*buffered {
        // SEEK_CUR is relative to stdio's logical position, not the end of
        // what it buffered
        if let std::io::SeekFrom::Current(_) = whence {
            let _ = f.seek(std::io::SeekFrom::Start(os_pos - stdio_held));
        }
        rbuf.clear();
        *rb = 0;
    }
    let new = f.seek(whence).unwrap_or(0);
    if *last_was_write {
        *wpos = new;
    } else {
        *frpos = new;
    }
    Ok(pos as f64)
}

// R: connections.c raw_seek
fn raw_seek(con: &mut Connection, r#where: f64, origin: i32) -> RResult<f64> {
    let ConnKind::Raw { pos, nbytes, .. } = &mut con.kind else { unreachable!("raw_seek on another kind") };
    let old = *pos;
    if r#where.is_nan() {
        return Ok(old as f64);
    }
    let newpos = match origin {
        2 => *pos as f64 + r#where,
        3 => *nbytes as f64 + r#where,
        _ => r#where,
    };
    if newpos < 0.0 || newpos > *nbytes as f64 {
        return Err(Flow::error("attempt to seek outside the range of the raw connection"));
    }
    *pos = newpos as usize;
    Ok(old as f64)
}

/// `seek(con, where, origin, rw)`
// R: connections.c do_seek
fn do_seek(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let v = arg(&args, 0);
    if !inherits_connection(&v) {
        return Err(Flow::error("'con' is not a connection"));
    }
    let n = con_arg(&v)?;
    if !i.conns.get(n)?.isopen {
        return Err(Flow::error("connection is not open"));
    }
    let r#where = crate::coerce::as_real(&arg(&args, 1));
    let origin = as_integer(&arg(&args, 2));
    let rw = as_integer(&arg(&args, 3));
    if !r#where.is_nan() {
        let con = i.conns.get(n)?;
        con.pushback.clear();
        con.pos_pushback = 0;
    }
    Ok(Value::real(con_seek(i, n, r#where, origin, rw)?))
}

/// `truncate(con)`
// R: connections.c do_truncate
fn do_truncate(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let v = arg(&args, 0);
    if !inherits_connection(&v) {
        return Err(Flow::error("'con' is not a connection"));
    }
    let n = con_arg(&v)?;
    con_truncate(i, n)?;
    Ok(Value::Null)
}

/// `con->truncate(con)`.
fn con_truncate(i: &mut Interp, n: i32) -> RResult<()> {
    let con = i.conns.get(n)?;
    match &mut con.kind {
        // R: connections.c raw_truncate
        ConnKind::Raw { pos, nbytes, .. } => {
            *nbytes = *pos;
        }
        // R: connections.c null_truncate
        ConnKind::File { fifo: true, .. } => {
            return Err(Flow::error("truncation not enabled for this connection"));
        }
        // R: connections.c file_truncate
        ConnKind::File { fp: Some(f), wbuf, last_was_write, .. } => {
            use std::io::Seek;
            if !con.isopen || !con.canwrite {
                return Err(Flow::error("can only truncate connections open for writing"));
            }
            let _ = flush_file_wbuf(f, wbuf);
            let size = f.stream_position().unwrap_or(0);
            if f.set_len(size).is_err() {
                return Err(Flow::error("file truncation failed"));
            }
            *last_was_write = true;
        }
        // R: connections.c null_truncate
        ConnKind::File { fp: None, .. }
        | ConnKind::Terminal
        | ConnKind::TextIn { .. }
        | ConnKind::TextOut { .. }
        | ConnKind::Gz { .. }
        | ConnKind::Url { .. }
        | ConnKind::Gzcon { .. }
        | ConnKind::Sock(_)
        | ConnKind::ServSock { .. } => return Err(Flow::error("truncation not enabled for this connection")),
        ConnKind::Native { .. } => return native_bridge()?.conn_truncate(i, n),
    }
    Ok(())
}

// ----------------------------------------------------------------------
// binary reading and writing

/// Read up to `buf.len()` bytes from a connection (R's `con->read`),
/// returning how many bytes were read.
pub fn con_read_bytes(i: &mut Interp, n: i32, buf: &mut [u8]) -> RResult<usize> {
    let con = i.conns.get(n)?;
    let mut used = 0usize;
    // Pushed-back bytes come first.
    while used < buf.len() && con.pos_pushback < con.pushback.len() {
        let last = con.pushback.len() - 1;
        let src = &con.pushback[last];
        let take = (buf.len() - used).min(src.len() - con.pos_pushback);
        buf[used..used + take].copy_from_slice(&src[con.pos_pushback..con.pos_pushback + take]);
        con.pos_pushback += take;
        used += take;
        if con.pos_pushback >= src.len() {
            con.pushback.pop();
            con.pos_pushback = 0;
        }
    }
    let used = unread_take(con, buf, used);
    if used == buf.len() {
        return Ok(used);
    }
    let k = con_read(con, buf, used)?;
    if k != NATIVE_READ {
        return Ok(k);
    }
    native_read(i, n, buf, used)
}

/// A native connection's `con->read` for [`con_read_bytes`].
#[cold]
#[inline(never)]
fn native_read(i: &mut Interp, n: i32, buf: &mut [u8], used: usize) -> RResult<usize> {
    if let ConnKind::Gz { xz: Some(x), .. } = &mut i.conns.get(n)?.kind {
        // R: connections.c:xzfile_read
        let (k, warn) = x.read(&mut buf[used..]);
        if let Some(w) = warn {
            i.warning(w)?;
        }
        return Ok(used + k);
    }
    Ok(used + native_bridge()?.conn_read(i, n, &mut buf[used..])?)
}

/// `con->read(buf + used, 1, buf.len() - used, con)`: `used` plus how many
/// bytes it read ([`NATIVE_READ`] for a native connection).
#[inline(always)]
fn con_read(con: &mut Connection, buf: &mut [u8], mut used: usize) -> RResult<usize> {
    file_switch_to_read(&mut con.kind);
    match &mut con.kind {
        // a failing xz decoder, which may warn: native_read (as R's
        // con->read, it passes a text connection's buffer by)
        ConnKind::Gz { xz: Some(_), .. } => Ok(NATIVE_READ),
        ConnKind::TextIn { data, cur } | ConnKind::Gz { data, cur, .. } => {
            let take = (buf.len() - used).min(data.len() - *cur);
            buf[used..used + take].copy_from_slice(&data[*cur..*cur + take]);
            *cur += take;
            Ok(used + take)
        }
        ConnKind::Gzcon { data, cur, .. } => {
            let take = (buf.len() - used).min(data.len() - *cur);
            buf[used..used + take].copy_from_slice(&data[*cur..*cur + take]);
            *cur += take;
            Ok(used + take)
        }
        ConnKind::Raw { data, pos, nbytes } => {
            let take = (buf.len() - used).min(nbytes.saturating_sub(*pos));
            buf[used..used + take].copy_from_slice(&data[*pos..*pos + take]);
            *pos += take;
            Ok(used + take)
        }
        ConnKind::File { fp: Some(f), rbuf, rpos, fifo, .. } => {
            // R: connections.c fifo_read — fifos read unbuffered.
            if !*fifo {
                let take = (buf.len() - used).min(rbuf.len() - *rpos);
                buf[used..used + take].copy_from_slice(&rbuf[*rpos..*rpos + take]);
                *rpos += take;
                used += take;
                if used == buf.len() {
                    return Ok(used);
                }
                rbuf.clear();
                *rpos = 0;
            }
            match f.read(&mut buf[used..]) {
                Ok(k) => Ok(used + k),
                Err(_) => Err(Flow::error("error reading from the connection")),
            }
        }
        ConnKind::Terminal => Ok(used + crate::console::read(&mut buf[used..])),
        ConnKind::Sock(s) => {
            let (k, incomplete) = s.read(&mut buf[used..], con.blocking);
            con.incomplete = incomplete;
            Ok(used + k)
        }
        // url-libcurl, fetched whole at open (url_open)
        ConnKind::Url { data, cur, .. } => {
            let take = (buf.len() - used).min(data.len().saturating_sub(*cur));
            buf[used..used + take].copy_from_slice(&data[*cur..*cur + take]);
            *cur += take;
            Ok(used + take)
        }
        // R: connections.c null_read
        ConnKind::File { fp: None, .. } | ConnKind::TextOut { .. } | ConnKind::ServSock { .. } => {
            Err(Flow::error("cannot read from this connection"))
        }
        ConnKind::Native { .. } => Ok(NATIVE_READ),
    }
}

/// Read one NUL-terminated string from a connection.
// R: connections.c readOneString
fn read_one_string(i: &mut Interp, n: i32) -> RResult<Option<RStr>> {
    let mut buf = Vec::new();
    loop {
        if buf.len() >= 10000 {
            i.warning("null terminator not found: breaking string at 10000 bytes")?;
            return Ok(Some(rstr(&String::from_utf8_lossy(&buf))));
        }
        let mut b = [0u8; 1];
        let got = con_read_bytes(i, n, &mut b)?;
        if got == 0 {
            if !buf.is_empty() {
                i.warning("incomplete string at end of file has been discarded")?;
            }
            return Ok(None);
        }
        if b[0] == 0 {
            return Ok(Some(rstr(&String::from_utf8_lossy(&buf))));
        }
        buf.push(b[0]);
    }
}

/// R's `rawOneString`: the rest of the vector as one string, with or
/// without a NUL terminator.
// R: connections.c rawOneString
fn raw_one_string(bytes: &[u8], np: &mut usize) -> RStr {
    if let Some(rel) = bytes[*np..].iter().position(|&b| b == 0) {
        let i = *np + rel;
        let s = rstr(&String::from_utf8_lossy(&bytes[*np..i]));
        *np = i + 1;
        s
    } else {
        let s = rstr(&String::from_utf8_lossy(&bytes[*np..]));
        *np = bytes.len();
        s
    }
}

/// `readBin(con, what, n, size, signed, swap)`
// R: connections.c do_readbin
fn do_readbin(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let con_arg0 = arg(&args, 0);
    let (mut ncon, bytes): (Option<i32>, Option<Vec<u8>>) = match &con_arg0 {
        Value::Raw(r) => (None, Some(r.data.clone())),
        v => {
            let n = con_arg(v)?;
            if i.conns.get(n)?.text {
                return Err(Flow::error("can only read from a binary connection"));
            }
            (Some(n), None)
        }
    };
    let what = match arg(&args, 1) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error("invalid 'what' argument")),
    };
    let n = crate::coerce::as_vec_size(i, &arg(&args, 2))?;
    if n < 0 {
        return Err(Flow::error("invalid 'n' argument"));
    }
    let n = n as usize;
    let mut size = as_integer(&arg(&args, 3));
    let signd = as_logical(&arg(&args, 4)) != 0;
    if as_logical(&arg(&args, 4)) == NA_LOGICAL {
        return Err(Flow::error("invalid 'signed' argument"));
    }
    let swap = as_logical(&arg(&args, 5)) != 0;
    if as_logical(&arg(&args, 5)) == NA_LOGICAL {
        return Err(Flow::error("invalid 'swap' argument"));
    }
    let mut np = 0usize;
    // A raw source reads from the vector; a connection is opened "rb" if
    // needed (documented behaviour).
    let mut wasopen = true;
    if let Some(nc) = ncon {
        wasopen = i.conns.get(nc)?.isopen;
        if !wasopen {
            let opened = open_in_mode(i, nc, "rb")?;
            if !opened {
                return Err(Flow::error("cannot open the connection"));
            }
            ncon = Some(nc);
        }
        if !i.conns.get(nc)?.canread {
            if !wasopen {
                close_conn(i, nc)?;
            }
            return Err(Flow::error("cannot read from this connection"));
        }
    }
    let r = (|| -> RResult<Value> {
        match what.as_str() {
            "character" => {
                let mut out: Vec<RStr> = Vec::new();
                for _ in 0..n {
                    match ncon {
                        Some(nc) => match read_one_string(i, nc)? {
                            Some(s) => out.push(s),
                            None => break,
                        },
                        // R: connections.c rawOneString never fails; past
                        // the end it yields empty strings.
                        None => {
                            let Some(b) = bytes.as_ref() else { break };
                            out.push(raw_one_string(b, &mut np));
                        }
                    }
                }
                Ok(Value::strs(out))
            }
            "complex" => {
                if size == NA_INTEGER {
                    size = 16;
                }
                if size != 16 {
                    return Err(Flow::error("size changing is not supported for complex vectors"));
                }
                let chunk = readbin_chunk(i, ncon, n, 16, 16);
                let mut reader = ChunkedReader::new(ncon, bytes.as_ref(), &mut np, readbin_short_end(i, ncon), chunk);
                let mut out = Vec::with_capacity(n);
                let mut b = [0u8; 16];
                for idx in 0..n {
                    if !reader.read_exact(i, n - idx, &mut b)? {
                        break;
                    }
                    let mut re = f64::from_le_bytes(b[..8].try_into().unwrap());
                    let mut im = f64::from_le_bytes(b[8..].try_into().unwrap());
                    if swap {
                        re = f64::from_bits(re.to_bits().swap_bytes());
                        im = f64::from_bits(im.to_bits().swap_bytes());
                    }
                    out.push(Complex::new(re, im));
                }
                Ok(Value::cplxs(out))
            }
            w if w == "integer" || w == "int" || w == "logical" || w == "raw" => {
                let (sizedef, is_logical, is_raw) = match w {
                    "logical" => (4, true, false),
                    "raw" => (1, false, true),
                    _ => (4, false, false),
                };
                if size == NA_INTEGER {
                    size = sizedef;
                }
                match size {
                    1 | 2 | 4 | 8 => {}
                    _ => return Err(Flow::error(format!("size {} is unknown on this machine", size))),
                }
                if is_raw && size != 1 {
                    return Err(Flow::error("raw is always of size 1"));
                }
                if !signd && (is_logical || size > 2) && !is_raw {
                    i.warning("'signed = FALSE' is only valid for integers of sizes 1 and 2")?;
                }
                if is_raw {
                    let mut raw_buf = vec![0u8; n];
                    let mut total_read = 0;
                    let short_end = readbin_short_end(i, ncon);
                    let block = readbin_block(i, ncon).unwrap_or(n);
                    while total_read < n {
                        let want = (n - total_read).min(block);
                        let got = readbin_read(
                            i,
                            ncon,
                            bytes.as_ref(),
                            &mut np,
                            &mut raw_buf[total_read..total_read + want],
                        )?;
                        total_read += got;
                        // R: connections.c:do_readbin — a short block ends
                        // the read (an xzfile warns at each that fails)
                        if got == 0 || (got < want && short_end) {
                            break;
                        }
                    }
                    raw_buf.truncate(total_read);
                    return Ok(Value::raws(raw_buf));
                }
                let sz = size as usize;
                let chunk = readbin_chunk(i, ncon, n, sz, sizedef as usize);
                let mut reader = ChunkedReader::new(ncon, bytes.as_ref(), &mut np, readbin_short_end(i, ncon), chunk);
                let mut ints: Vec<i32> = Vec::with_capacity(n);
                let mut b = [0u8; 8];
                for idx in 0..n {
                    if !reader.read_exact(i, n - idx, &mut b[..sz])? {
                        break;
                    }
                    let v = read_int_bytes(&b, size, signd, swap);
                    ints.push(v);
                }
                Ok(if is_logical { Value::lgls(ints) } else { Value::ints(ints) })
            }
            w if w == "numeric" || w == "double" => {
                if size == NA_INTEGER {
                    size = 8;
                }
                match size {
                    4 | 8 | 16 => {}
                    _ => return Err(Flow::error(format!("size {} is unknown on this machine", size))),
                }
                let sz = size as usize;
                let chunk = readbin_chunk(i, ncon, n, sz, 8);
                let mut reader = ChunkedReader::new(ncon, bytes.as_ref(), &mut np, readbin_short_end(i, ncon), chunk);
                let mut out = Vec::with_capacity(n);
                let mut b = [0u8; 16];
                for idx in 0..n {
                    if !reader.read_exact(i, n - idx, &mut b[..sz])? {
                        break;
                    }
                    out.push(read_real_bytes(&b, size, swap));
                }
                Ok(Value::reals(out))
            }
            _ => Err(Flow::error("invalid 'what' argument")),
        }
    })();
    if let Some(nc) = ncon {
        if !wasopen {
            close_conn(i, nc)?;
        }
    }
    r
}

/// Whether readBin's source ends at its first short read (see
/// [`Connection::short_read_is_end`]); a raw vector does.
fn readbin_short_end(i: &mut Interp, ncon: Option<i32>) -> bool {
    ncon.is_none_or(|n| i.conns.get(n).is_ok_and(|c| c.short_read_is_end()))
}

/// R: connections.c:BLOCK — how many items readBin asks of `con->read` at
/// a time.
const READBIN_BLOCK: usize = 8096;

/// The items per `con->read` of readBin on connection `ncon` where how its
/// reads are cut can be seen: a failing xz decoder ([`XzLive`]) warns at
/// each read that meets the failure. Elsewhere robigo cuts them as it
/// likes.
// R: connections.c:do_readbin
fn readbin_block(i: &mut Interp, ncon: Option<i32>) -> Option<usize> {
    let n = ncon?;
    matches!(i.conns.get(n).ok()?.kind, ConnKind::Gz { xz: Some(_), .. }).then_some(READBIN_BLOCK)
}

/// The bytes per read of a [`ChunkedReader`] for `n` items of `size`
/// bytes: R's (blocks of [`READBIN_BLOCK`] items, one item when `size` is
/// not the type's `sizedef`) where they can be seen ([`readbin_block`]),
/// else 8192 bytes.
// R: connections.c:do_readbin
fn readbin_chunk(i: &mut Interp, ncon: Option<i32>, n: usize, size: usize, sizedef: usize) -> usize {
    let chunk = match readbin_block(i, ncon) {
        Some(block) if size == sizedef => block * size,
        Some(_) => size,
        None => 8192,
    };
    chunk.min(n.saturating_mul(size))
}

/// Read into `buf` from a raw vector (advancing `np`) or a connection.
// R: connections.c rawRead / con->read
fn readbin_read(
    i: &mut Interp,
    ncon: Option<i32>,
    bytes: Option<&Vec<u8>>,
    np: &mut usize,
    buf: &mut [u8],
) -> RResult<usize> {
    match (ncon, bytes) {
        (Some(nc), _) => con_read_bytes(i, nc, buf),
        (None, Some(b)) => {
            let take = buf.len().min(b.len().saturating_sub(*np));
            buf[..take].copy_from_slice(&b[*np..*np + take]);
            *np += take;
            Ok(take)
        }
        _ => Ok(0),
    }
}

struct ChunkedReader<'a> {
    ncon: Option<i32>,
    bytes: Option<&'a Vec<u8>>,
    np: &'a mut usize,
    /// The bytes of a read: as many as each read asks for.
    buf: Vec<u8>,
    pos: usize,
    len: usize,
    eof: bool,
    /// The source ends at its first short read.
    short_end: bool,
}

impl<'a> ChunkedReader<'a> {
    fn new(ncon: Option<i32>, bytes: Option<&'a Vec<u8>>, np: &'a mut usize, short_end: bool, chunk: usize) -> Self {
        Self { ncon, bytes, np, buf: vec![0u8; chunk], pos: 0, len: 0, eof: false, short_end }
    }

    fn read_exact(&mut self, i: &mut Interp, remaining_items: usize, out: &mut [u8]) -> RResult<bool> {
        let needed = out.len();
        let mut written = 0;
        while written < needed {
            let available = self.len - self.pos;
            if available > 0 {
                let take = (needed - written).min(available);
                out[written..written + take].copy_from_slice(&self.buf[self.pos..self.pos + take]);
                self.pos += take;
                written += take;
            } else if self.eof {
                return Ok(false);
            } else {
                self.pos = 0;
                let remaining_bytes = remaining_items.saturating_mul(needed).saturating_sub(written);
                debug_assert!(needed <= self.buf.len(), "element size must fit the chunk buffer");
                let max_read = remaining_bytes.min(self.buf.len());
                let got = readbin_read(i, self.ncon, self.bytes, self.np, &mut self.buf[..max_read])?;
                if got == 0 {
                    self.eof = true;
                    self.len = 0;
                    return Ok(false);
                }
                self.len = got;
                // R: connections.c:do_readbin — a short block ends the read
                self.eof = got < max_read && self.short_end;
            }
        }
        Ok(true)
    }
}

/// Decode an integer of `size` (1, 2, 4 or 8) bytes, native endianness.
fn read_int_bytes(b: &[u8; 8], size: i32, signd: bool, swap: bool) -> i32 {
    let size = size as usize;
    let mut raw = [0u8; 8];
    raw[..size].copy_from_slice(&b[..size]);
    if swap && size > 1 {
        raw[..size].reverse();
    }
    match (size, signd) {
        (1, true) => raw[0] as i8 as i32,
        (1, false) => raw[0] as i32,
        (2, true) => i16::from_le_bytes(raw[..2].try_into().unwrap()) as i32,
        (2, false) => u16::from_le_bytes(raw[..2].try_into().unwrap()) as i32,
        (4, _) => i32::from_le_bytes(raw[..4].try_into().unwrap()),
        (8, _) => i64::from_le_bytes(raw[..8].try_into().unwrap()) as i32,
        _ => 0,
    }
}

/// Decode a double of `size` (4, 8 or 16 x87) bytes, native endianness.
fn read_real_bytes(b: &[u8], size: i32, swap: bool) -> f64 {
    let size = size as usize;
    // R: connections.c R_ReadBin's sizeof(long double) case: the 16 bytes
    // are swapped as a whole, then converted from the x87 80-bit format.
    if size == 16 {
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&b[..16]);
        if swap {
            raw.reverse();
        }
        return crate::nmath::ldouble::LDouble::from_bytes(raw).to_f64();
    }
    let mut raw = [0u8; 8];
    raw[..size.min(8)].copy_from_slice(&b[..size.min(8)]);
    if swap && size > 1 {
        raw[..size.min(8)].reverse();
    }
    match size {
        4 => f32::from_le_bytes(raw[..4].try_into().unwrap()) as f64,
        _ => f64::from_le_bytes(raw),
    }
}

/// `writeBin(object, con, size, swap, useBytes)`
// R: connections.c do_writebin
fn do_writebin(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let _swap_raw = as_logical(&arg(&args, 3));
    let object = arg(&args, 0);
    if !object.is_atomic() {
        return Err(Flow::error("'x' is not an atomic vector type"));
    }
    let con_arg1 = arg(&args, 1);
    let (mut ncon, is_raw) = match &con_arg1 {
        Value::Raw(_) => (None, true),
        v => {
            let n = con_arg(v)?;
            if i.conns.get(n)?.text {
                return Err(Flow::error("can only write to a binary connection"));
            }
            if !i.conns.get(n)?.canwrite {
                return Err(Flow::error("cannot write to this connection"));
            }
            (Some(n), false)
        }
    };
    let mut size = as_integer(&arg(&args, 2));
    let swap = as_logical(&arg(&args, 3)) != 0;
    if as_logical(&arg(&args, 3)) == NA_LOGICAL {
        return Err(Flow::error("invalid 'swap' argument"));
    }
    let use_bytes = as_logical(&arg(&args, 4));
    if use_bytes == NA_LOGICAL {
        return Err(Flow::error("invalid 'useBytes' argument"));
    }
    let len = object.len();
    if len == 0 {
        i.visible = is_raw;
        return Ok(if is_raw { Value::raws(vec![]) } else { Value::Null });
    }
    let mut wasopen = true;
    if let Some(nc) = ncon {
        wasopen = i.conns.get(nc)?.isopen;
        if !wasopen {
            let opened = open_in_mode(i, nc, "wb")?;
            if !opened {
                return Err(Flow::error("cannot open the connection"));
            }
            ncon = Some(nc);
            if !i.conns.get(nc)?.canwrite {
                close_conn(i, nc)?;
                return Err(Flow::error("cannot write to this connection"));
            }
        }
    }
    let r = (|| -> RResult<Vec<u8>> {
        if let Value::Str(s) = &object {
            // Strings are written NUL-terminated.
            let mut out = Vec::new();
            for e in &s.data {
                let b = e.as_deref().unwrap_or("NA").as_bytes();
                out.extend_from_slice(b);
                out.push(0);
            }
            return Ok(out);
        }
        match &object {
            Value::Lgl(x) | Value::Int(x) => {
                if size == NA_INTEGER {
                    size = 4;
                }
                match size {
                    1 | 2 | 4 | 8 => {}
                    _ => return Err(Flow::error(format!("size {} is unknown on this machine", size))),
                }
                let mut out = vec![0u8; len * size as usize];
                for (k, &v) in x.data.iter().enumerate() {
                    let b = match size {
                        1 => vec![v as i8 as u8],
                        2 => (v as i16).to_le_bytes().to_vec(),
                        4 => v.to_le_bytes().to_vec(),
                        _ => (v as i64).to_le_bytes().to_vec(),
                    };
                    out[k * size as usize..(k + 1) * size as usize].copy_from_slice(&b);
                }
                if swap && size > 1 {
                    for chunk in out.chunks_mut(size as usize) {
                        chunk.reverse();
                    }
                }
                Ok(out)
            }
            Value::Real(x) => {
                if size == NA_INTEGER {
                    size = 8;
                }
                match size {
                    4 | 8 | 16 => {}
                    _ => return Err(Flow::error(format!("size {} is unknown on this machine", size))),
                }
                let mut out = vec![0u8; len * size as usize];
                for (k, &v) in x.data.iter().enumerate() {
                    let b: Vec<u8> = if size == 4 {
                        (v as f32).to_le_bytes().to_vec()
                    } else if size == 8 {
                        v.to_le_bytes().to_vec()
                    } else {
                        // R: connections.c do_writebin's sizeof(long double):
                        // (long double) x into a zero-initialized static, of
                        // which the store writes the x87 80-bit first 10 bytes.
                        crate::nmath::ldouble::LDouble::from_f64(v).to_bytes().to_vec()
                    };
                    out[k * size as usize..(k + 1) * size as usize].copy_from_slice(&b);
                }
                if swap && size > 1 {
                    for chunk in out.chunks_mut(size as usize) {
                        chunk.reverse();
                    }
                }
                Ok(out)
            }
            Value::Cplx(x) => {
                if size == NA_INTEGER {
                    size = 16;
                }
                if size != 16 {
                    return Err(Flow::error("size changing is not supported for complex vectors"));
                }
                let mut out = vec![0u8; len * 16];
                for (k, z) in x.data.iter().enumerate() {
                    out[k * 16..k * 16 + 8].copy_from_slice(&z.re.to_le_bytes());
                    out[k * 16 + 8..(k + 1) * 16].copy_from_slice(&z.im.to_le_bytes());
                }
                if swap {
                    for chunk in out.chunks_mut(8) {
                        chunk.reverse();
                    }
                }
                Ok(out)
            }
            Value::Raw(x) => {
                if size == NA_INTEGER {
                    size = 1;
                }
                if size != 1 {
                    return Err(Flow::error("size changing is not supported for raw vectors"));
                }
                Ok(x.data.clone())
            }
            _ => Err(Flow::error("'x' is not an atomic vector type")),
        }
    })();
    let out = r;
    let write_res = if let (Some(nc), Ok(b)) = (ncon, out.as_ref()) { con_write_bytes(i, nc, b) } else { Ok(()) };
    if let Some(nc) = ncon {
        if !wasopen {
            check_close(i, nc)?;
        }
    }
    write_res?;
    // R: connections.c do_writebin — visible iff the target was a raw
    // vector (whose contents it returns).
    i.visible = is_raw_target(&con_arg1);
    match out {
        Ok(b) => Ok(if is_raw_target(&con_arg1) { Value::raws(b) } else { Value::Null }),
        Err(e) => Err(e),
    }
}

fn is_raw_target(v: &Value) -> bool {
    matches!(v, Value::Raw(_))
}

// ----------------------------------------------------------------------
// fixed-size character reading and writing

/// `readChar(con, nchars, useBytes)` — `con` may be a raw vector.
// R: connections.c do_readchar
fn do_readchar(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let con_arg0 = arg(&args, 0);
    let (ncon, bytes): (Option<i32>, Option<Vec<u8>>) = match &con_arg0 {
        Value::Raw(r) => (None, Some(r.data.clone())),
        v => {
            let n = con_arg(v)?;
            if !i.conns.get(n)?.canread {
                return Err(Flow::error("cannot read from this connection"));
            }
            (Some(n), None)
        }
    };
    let nchars = match arg(&args, 1) {
        Value::Int(v) => v.data.clone(),
        _ => return Err(Flow::error("invalid 'nchars' argument")),
    };
    let n = nchars.len();
    if n == 0 {
        return Ok(Value::strs(vec![]));
    }
    let use_bytes = as_logical(&arg(&args, 2));
    if use_bytes == NA_LOGICAL {
        return Err(Flow::error("invalid 'useBytes' argument"));
    }
    let mut wasopen = true;
    if let Some(nc) = ncon {
        wasopen = i.conns.get(nc)?.isopen;
        if !wasopen {
            let opened = open_in_mode(i, nc, "rb")?;
            if !opened {
                return Err(Flow::error("cannot open the connection"));
            }
        }
        if !i.conns.get(nc)?.canread {
            if !wasopen {
                close_conn(i, nc)?;
            }
            return Err(Flow::error("cannot read from this connection"));
        }
        if i.conns.get(nc)?.text {
            i.warning("text connection used with readChar(), results may be incorrect")?;
        }
    }

    let mut np = 0usize;
    let mut warn_on_nul = true;
    let mut out: Vec<RStr> = Vec::with_capacity(n);
    let r = (|| -> RResult<()> {
        for &len in &nchars {
            if len == NA_INTEGER || len < 0 {
                return Err(Flow::error("invalid 'nchars' argument"));
            }
            let len = len as usize;
            match (&bytes, ncon) {
                (Some(b), _) => {
                    // R: connections.c rawFixedString
                    let mut l = len;
                    if np + l > b.len() {
                        l = b.len().saturating_sub(np);
                        if l == 0 {
                            break;
                        }
                    }
                    let s = String::from_utf8_lossy(&b[np..np + l]).into_owned();
                    np += l;
                    out.push(rstr(&s));
                }
                (None, Some(nc)) => {
                    // R: connections.c readFixedString
                    let mut buf = vec![0u8; len];
                    let got = con_read_bytes(i, nc, &mut buf)?;
                    if len > 0 && got == 0 {
                        break;
                    }
                    buf.truncate(got);
                    if buf.len() < got {
                        // unreachable, keeps clippy quiet
                    }
                    if buf.contains(&0) && warn_on_nul {
                        warn_on_nul = false;
                        i.warning("truncating string with embedded nuls")?;
                    }
                    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                    out.push(rstr(&String::from_utf8_lossy(&buf[..end])));
                }
                _ => break,
            }
        }
        Ok(())
    })();
    if let Some(nc) = ncon {
        if !wasopen {
            close_conn(i, nc)?;
        }
    }
    r?;
    Ok(Value::strs(out))
}

/// `writeChar(object, con, nchars, eos, useBytes)`
// R: connections.c do_writechar
fn do_writechar(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let object = match arg(&args, 0) {
        Value::Str(s) => s.data.clone(),
        _ => return Err(Flow::error("invalid 'object' argument")),
    };
    let con_arg1 = arg(&args, 1);
    let (mut ncon, is_raw) = match &con_arg1 {
        Value::Raw(_) => (None, true),
        v => {
            let n = con_arg(v)?;
            if !i.conns.get(n)?.canwrite {
                return Err(Flow::error("cannot write to this connection"));
            }
            (Some(n), false)
        }
    };
    let nchars = match arg(&args, 2) {
        Value::Int(v) => v.data.clone(),
        _ => return Err(Flow::error("invalid 'nchars' argument")),
    };
    let sep = arg(&args, 3);
    let ssep = match &sep {
        Value::Null => None,
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().map(|s| s.to_string()),
        _ => return Err(Flow::error("invalid 'sep' argument")),
    };
    let use_bytes = as_logical(&arg(&args, 4));
    if use_bytes == NA_LOGICAL {
        return Err(Flow::error("invalid 'useBytes' argument"));
    }
    let n = nchars.len();
    if object.len() < n {
        return Err(Flow::error("'object' is too short"));
    }
    if n == 0 {
        i.visible = is_raw;
        return Ok(if is_raw { Value::raws(vec![]) } else { Value::Null });
    }
    let mut wasopen = true;
    if let Some(nc) = ncon {
        wasopen = i.conns.get(nc)?.isopen;
        if !wasopen {
            let opened = open_in_mode(i, nc, "wb")?;
            if !opened {
                return Err(Flow::error("cannot open the connection"));
            }
            ncon = Some(nc);
            if !i.conns.get(nc)?.canwrite {
                close_conn(i, nc)?;
                return Err(Flow::error("cannot write to this connection"));
            }
        }
    }
    let mut out: Vec<u8> = Vec::new();
    let r = (|| -> RResult<()> {
        for (k, &len) in nchars.iter().enumerate() {
            if len == NA_INTEGER || len < 0 {
                return Err(Flow::error("invalid 'nchars' argument"));
            }
            let len = len as usize;
            let s = object[k].as_deref().unwrap_or("NA");
            let sb = s.as_bytes();
            let sepl = ssep.as_ref().map(|x| x.len() + 1).unwrap_or(0);
            let start = out.len();
            if sb.contains(&0) {
                // Embedded nuls: operate byte-wise (R's first branch).
                if len > sb.len() {
                    i.warning("writeChar: more bytes requested than are in the string - will zero-pad")?;
                }
                out.resize(start + len + sepl, 0);
                let take = sb.len().min(len);
                out[start..start + take].copy_from_slice(&sb[..take]);
            } else {
                // R: lenb starts at the byte length; nchars counts characters.
                let lenc = s.chars().count();
                let mut lenb = sb.len();
                if len > lenc {
                    i.warning("writeChar: more characters requested than are in the string - will zero-pad")?;
                    lenb += len - lenc;
                } else if len < lenc {
                    lenb = s.chars().take(len).map(|c| c.len_utf8()).sum();
                }
                out.resize(start + lenb + sepl, 0);
                let take = sb.len().min(lenb);
                out[start..start + take].copy_from_slice(&sb[..take]);
            }
            if let Some(sep) = &ssep {
                let p = out.len() - sep.len() - 1;
                out[p..p + sep.len()].copy_from_slice(sep.as_bytes());
            }
        }
        Ok(())
    })();
    let write_res = if let Some(nc) = ncon { if r.is_ok() { con_write_bytes(i, nc, &out) } else { r } } else { r };
    if let Some(nc) = ncon {
        if !wasopen {
            check_close(i, nc)?;
        }
    }
    write_res?;
    // R: connections.c do_writechar — visible iff the target was a raw
    // vector.
    i.visible = is_raw;
    if is_raw { Ok(Value::raws(out)) } else { Ok(Value::Null) }
}

// ----------------------------------------------------------------------
// cat

/// A terminal connection object: integer with class c("terminal", "connection").
pub fn terminal_connection(n: i32) -> Value {
    let mut v = Value::int(n);
    v.set_attr_raw(s::CLASS, Value::strs_from(["terminal", "connection"]));
    v
}

/// Write to a connection number (1 = stdout, 2 = stderr, or any open
/// connection).
pub fn write_connection(i: &mut Interp, con: i32, text: &str) -> RResult<()> {
    conn_write(i, con, text)
}

/// `EncodeElement0(x, i, quote = 0)` as used by cat.
pub fn encode_element_cat(pp: &PrintParams, v: &Value, k: usize) -> String {
    match v {
        Value::Lgl(x) => encode_logical(x.data[k], 0, "NA"),
        Value::Int(x) => encode_integer(x.data[k], 0, "NA"),
        Value::Real(x) => {
            let (w, d, e) = format_real(&x.data[k..k + 1], pp.digits, pp.scipen, 0, pp.na_width);
            encode_real_dec(x.data[k], w, d, e, "NA", &pp.out_dec)
        }
        Value::Cplx(x) => {
            let z = x.data[k];
            let (r, im) = format_complex(&[z], pp.digits, pp.scipen, 0, pp.na_width);
            encode_complex_dec(z, r, im, "NA", &pp.out_dec)
        }
        Value::Str(x) => x.data[k].as_deref().unwrap_or("NA").to_string(),
        Value::Raw(x) => format!("{:02x}", x.data[k]),
        _ => String::new(),
    }
}

/// Write `cat()` output to connection `ifile` the way R does: by
/// temporarily diverting `Rprintf` output to it (`switch_stdout`).
fn cat_output(i: &mut Interp, ifile: i32, text: &str) -> RResult<()> {
    if text.is_empty() {
        return Ok(());
    }
    if ifile == i.conns.output_con {
        i.write_out(text);
        return Ok(());
    }
    conn_write(i, ifile, text)
}

/// A string as `cat` writes it (R: builtin.c:trChar): as it is, unless
/// its bytes are not its text (bytes, latin1, invalid) or the locale is
/// not UTF-8.
#[inline]
fn cat_str(e: &crate::value::RString) -> RResult<std::borrow::Cow<'_, str>> {
    if crate::locale::utf8locale() && !e.has_foreign_bytes() {
        Ok(std::borrow::Cow::Borrowed(e))
    } else {
        cat_str_translated(e)
    }
}

#[cold]
#[inline(never)]
fn cat_str_translated(e: &crate::value::RString) -> RResult<std::borrow::Cow<'_, str>> {
    Ok(std::borrow::Cow::Owned(crate::locale::display_native(e)?))
}

/// `strlen` of the native bytes of `cat` text, which `fill` counts.
#[inline]
fn native_len(p: &str) -> usize {
    crate::locale::output_bytes(p).len()
}

/// R: builtin.c:do_cat
fn do_cat(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let objs = match arg(&args, 0) {
        Value::List(l) => l.data.clone(),
        _ => vec![],
    };
    let ifile = as_integer(&arg(&args, 1));
    if !i.conns.get(ifile)?.canwrite {
        return Err(Flow::error("cannot write to this connection"));
    }
    let sepr: Vec<String> = match arg(&args, 2) {
        Value::Str(v) => v
            .data
            .iter()
            .map(|x| x.as_ref().map_or(Ok("NA".into()), |x| Ok(cat_str(x)?.into_owned())))
            .collect::<RResult<_>>()?,
        _ => return Err(Flow::error("invalid 'sep' specification")),
    };
    let nlsep = sepr.iter().any(|x| x.contains('\n'));
    let fill = arg(&args, 3);
    if !matches!(fill, Value::Lgl(_) | Value::Int(_) | Value::Real(_)) || fill.len() != 1 {
        return Err(Flow::error("invalid 'fill' argument"));
    }
    let pwidth: usize = match &fill {
        Value::Lgl(_) => {
            if as_logical(&fill) == TRUE {
                i.print_params().width
            } else {
                usize::MAX
            }
        }
        _ => {
            let w = as_integer(&fill);
            if w <= 0 {
                i.warning("non-positive 'fill' argument will be ignored")?;
                usize::MAX
            } else {
                w as usize
            }
        }
    };
    let labs: Vec<String> = match arg(&args, 4) {
        Value::Str(v) => v
            .data
            .iter()
            .map(|x| x.as_ref().map_or(Ok("NA".into()), |x| Ok(cat_str(x)?.into_owned())))
            .collect::<RResult<_>>()?,
        Value::Null => vec![],
        _ => return Err(Flow::error("invalid 'labels' argument")),
    };
    if as_logical(&arg(&args, 5)) == NA_LOGICAL {
        return Err(Flow::error("invalid 'append' specification"));
    }
    // R: switch_stdout(ifile, 0) opens a closed connection for the call.
    let mut opened_here = false;
    if ifile >= 3 && !i.conns.get(ifile)?.isopen {
        let con = i.conns.get(ifile)?;
        let mode = std::mem::replace(&mut con.mode, "wt".into());
        let ok = con_open(i, ifile)?;
        i.conns.get(ifile)?.mode = mode;
        if !ok {
            return Err(Flow::error("cannot open the connection"));
        }
        if !i.conns.get(ifile)?.canwrite {
            con_close_raw(i, ifile)?;
            return Err(Flow::error("cannot write to this connection"));
        }
        opened_here = true;
    }
    let mut out = String::new();
    let r = cat_body(i, &objs, &sepr, nlsep, pwidth, &fill, &labs, &mut out);
    // Output produced before an error is still written, as R prints as
    // it goes.
    let w = cat_output(i, ifile, &out);
    if opened_here {
        check_close(i, ifile)?;
    }
    r?;
    w?;
    Ok(Value::Null)
}

#[allow(clippy::too_many_arguments)]
fn cat_body(
    i: &mut Interp,
    objs: &[Value],
    sepr: &[String],
    nlsep: bool,
    pwidth: usize,
    fill: &Value,
    labs: &[String],
    out: &mut String,
) -> RResult<()> {
    let sep_at = |k: usize| -> &str {
        match sepr.len() {
            0 => "",
            1 => &sepr[0],
            n => &sepr[k % n],
        }
    };
    let sep_width = |k: usize| -> usize { crate::format::str_width(sep_at(k)) };
    let mut width = 0usize;
    let mut ntot = 0usize;
    let mut nlines = 0usize;
    let newline = |out: &mut String, width: &mut usize, nlines: usize| {
        out.push('\n');
        *width = 0;
        if !labs.is_empty() {
            let l = &labs[nlines % labs.len()];
            out.push_str(l);
            out.push(' ');
            *width += crate::format::str_width(l) + 1;
        }
    };
    // One PrintParams for the whole call, and only for numbers:
    // `encode_element_cat` used to build it per element, costing five option
    // lookups per cat() element.
    let pp = std::cell::OnceCell::new();
    if pwidth == usize::MAX {
        out.reserve(
            objs.iter()
                .map(|o| {
                    if let Value::Str(x) = o {
                        x.data.iter().map(|e| e.as_ref().map_or(2, |e| e.len()) + 1).sum()
                    } else {
                        0
                    }
                })
                .sum(),
        );
    }
    for (iobj, s0) in objs.iter().enumerate() {
        if iobj != 0 && !s0.is_null() {
            out.push_str(sep_at(ntot));
            ntot += 1;
        }
        let n = s0.len();
        if n == 0 {
            continue;
        }
        // Without filling, widths never matter: strings go out as they are.
        if let (Value::Str(x), usize::MAX, true) = (s0, pwidth, labs.is_empty()) {
            let utf8 = crate::locale::utf8locale();
            for (k, e) in x.data.iter().enumerate() {
                match e {
                    Some(e) if utf8 && !e.has_foreign_bytes() => out.push_str(e),
                    Some(e) => out.push_str(&cat_str_translated(e)?),
                    None => out.push_str("NA"),
                }
                if k < n - 1 {
                    out.push_str(sep_at(ntot));
                    ntot += 1;
                }
            }
            continue;
        }
        if !labs.is_empty() && iobj == 0 && as_integer(fill) > 0 {
            let l = &labs[nlines % labs.len()];
            out.push_str(l);
            out.push(' ');
            width += crate::format::str_width(l) + 1;
            nlines += 1;
        }
        // (Strings as they are, without a copy per element.)
        let elem = |k: usize| -> RResult<std::borrow::Cow<'_, str>> {
            match s0 {
                Value::Str(x) => x.data[k].as_ref().map_or(Ok(std::borrow::Cow::Borrowed("NA")), cat_str),
                Value::Sym(sy) => Ok(std::borrow::Cow::Owned(sy.name().to_string())),
                v if v.is_atomic() => {
                    Ok(std::borrow::Cow::Owned(encode_element_cat(pp.get_or_init(|| i.print_params()), v, k)))
                }
                v => Err(Flow::error(format!(
                    "argument {} (type '{}') cannot be handled by 'cat'",
                    iobj + 1,
                    v.type_name()
                ))),
            }
        };
        let mut p = elem(0)?;
        let mut w = native_len(&p);
        let mut sepw = sep_width(ntot);
        if iobj > 0 && width + w + sepw > pwidth {
            newline(out, &mut width, nlines);
            nlines += 1;
        }
        for k in 0..n {
            out.push_str(&p);
            width += w + sepw;
            if k < n - 1 {
                out.push_str(sep_at(ntot));
                p = elem(k + 1)?;
                w = native_len(&p);
                sepw = sep_width(ntot);
                if width + w + sepw > pwidth {
                    newline(out, &mut width, nlines);
                    nlines += 1;
                }
                ntot += 1;
            }
        }
    }
    if pwidth != usize::MAX || nlsep {
        out.push('\n');
    }
    Ok(())
}

/// R: connections.c:file_open — reading starts at 0, writing where the
/// file was opened; a connection that cannot read starts as writing.
fn init_file_positions(con: &mut Connection) {
    let (canread, canwrite) = (con.canread, con.canwrite);
    if let ConnKind::File { fp: Some(f), last_was_write, frpos, wpos, .. } = &mut con.kind {
        use std::io::Seek;
        *last_was_write = !canread;
        *frpos = 0;
        if canwrite {
            *wpos = f.stream_position().unwrap_or(0);
        }
    }
}

/// R: connections.c:file_fgetc_internal/file_read — before reading after
/// a write: remember the write position, go back to the read position.
#[inline]
fn file_switch_to_read(kind: &mut ConnKind) {
    if let ConnKind::File { last_was_write: true, .. } = kind {
        if let ConnKind::File {
            fp: Some(f),
            rbuf,
            rpos,
            wbuf,
            last_was_write,
            frpos,
            wpos,
            fifo: false,
            buffered,
            ..
        } = kind
        {
            use std::io::{Seek, SeekFrom};
            let _ = flush_file_wbuf(f, wbuf);
            *wpos = f.stream_position().unwrap_or(*wpos);
            *last_was_write = false;
            let _ = f.seek(SeekFrom::Start(*frpos));
            // R's con->buff outlives the switch; stdio's buffer does not.
            if !*buffered {
                rbuf.clear();
                *rpos = 0;
            }
        }
    }
}

/// [`file_switch_to_read`] for a read that refills `rbuf`: R's con->buff
/// is read first (buff_fgetc), and only its refill calls file_read, so a
/// buffered connection keeps writing until what it holds is used up.
#[inline]
fn file_switch_to_refill(kind: &mut ConnKind) {
    if let ConnKind::File { buffered: true, rbuf, rpos, .. } = kind {
        if *rpos < rbuf.len() {
            return;
        }
    }
    file_switch_to_read(kind);
}

/// R: connections.c:file_write — before writing after a read: remember
/// the read position (less what is buffered), go to the write position.
#[inline]
fn file_switch_to_write(kind: &mut ConnKind) {
    if let ConnKind::File { last_was_write: false, .. } = kind {
        if let ConnKind::File { fp: Some(f), rbuf, rpos, last_was_write, frpos, wpos, fifo: false, buffered, .. } = kind
        {
            use std::io::{Seek, SeekFrom};
            // R's con->buff keeps what it read past rpos (file_write takes
            // the raw f_tell); stdio's buffer is dropped.
            let held = if *buffered { 0 } else { (rbuf.len() - *rpos) as u64 };
            *frpos = f.stream_position().unwrap_or(0).saturating_sub(held);
            *last_was_write = true;
            let _ = f.seek(SeekFrom::Start(*wpos));
            if !*buffered {
                rbuf.clear();
                *rpos = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_write_buffered_accumulates_and_flushes() {
        let path = std::env::temp_dir().join(format!("robigo_test_io_{}", std::process::id()));
        let mut tmp = std::fs::File::create(&path).unwrap();
        let mut wbuf = Vec::new();
        // Writing 100 bytes buffers without OS write
        let data1 = vec![42u8; 100];
        file_write_buffered(&mut tmp, &mut wbuf, false, &data1).unwrap();
        assert_eq!(wbuf.len(), 100);
        assert_eq!(tmp.metadata().unwrap().len(), 0);

        // Flushing writes to underlying file
        flush_file_wbuf(&mut tmp, &mut wbuf).unwrap();
        assert!(wbuf.is_empty());
        assert_eq!(tmp.metadata().unwrap().len(), 100);

        // Writing exceeding FILE_WBUF_SIZE flushes buffer
        let data2 = vec![1u8; FILE_WBUF_SIZE - 50];
        file_write_buffered(&mut tmp, &mut wbuf, false, &data2).unwrap();
        assert_eq!(wbuf.len(), FILE_WBUF_SIZE - 50);

        let data3 = vec![2u8; 100];
        file_write_buffered(&mut tmp, &mut wbuf, false, &data3).unwrap();
        // data2 flushed, data3 buffered in wbuf
        assert_eq!(wbuf.len(), 100);
        assert_eq!(tmp.metadata().unwrap().len(), 100 + (FILE_WBUF_SIZE - 50) as u64);

        // Fifo bypasses buffer
        let data4 = vec![3u8; 50];
        file_write_buffered(&mut tmp, &mut wbuf, true, &data4).unwrap();
        assert_eq!(tmp.metadata().unwrap().len(), 100 + (FILE_WBUF_SIZE - 50) as u64 + 50);
        assert_eq!(wbuf.len(), 100); // unaffected

        drop(tmp);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn scan_buffered_discards_spaces_for_numeric() {
        let mut con = Connection::new("raw", "raw", "r", ConnKind::Raw { data: b"1 2,3".to_vec(), pos: 0, nbytes: 5 });
        let mut buf = Vec::new();
        let filled = con.scan_buffered(b',' as i32, false, true, &mut buf);
        assert_eq!(buf, b"12");
        assert_eq!(filled, b',' as i32);

        let filled2 = con.scan_buffered(b',' as i32, false, true, &mut buf);
        assert_eq!(buf, b"3");
        assert_eq!(filled2, EOF);
    }

    #[test]
    fn read_buffered_skips_nul_without_warning() {
        let mut con =
            Connection::new("raw", "raw", "r", ConnKind::Raw { data: vec![65, 0, 66, 10], pos: 0, nbytes: 4 });
        let rl = ReadLines { nnn: 10, skip_nul: true, warn: true, oenc: Encoding::Native };
        let mut ans = Vec::new();
        let mut nul_warnings = Vec::new();
        let mut incomplete = None;
        let eof = rl.read_buffered(&mut con, &mut ans, &mut nul_warnings, &mut incomplete);
        if eof && incomplete.is_none() {
            ans.pop();
        }
        assert_eq!(ans.len(), 1);
        assert_eq!(ans[0].as_deref().unwrap(), "AB");
        assert!(nul_warnings.is_empty());
    }

    #[test]
    fn read_buffered_handles_consecutive_cr() {
        let mut con = Connection::new(
            "textConnection",
            "test",
            "r",
            ConnKind::TextIn { data: b"foo\r\r\nfoo\n".to_vec(), cur: 0 },
        );
        let rl = ReadLines { nnn: 10, skip_nul: false, warn: true, oenc: Encoding::Native };
        let mut ans = Vec::new();
        let mut nul_warnings = Vec::new();
        let mut incomplete = None;
        let eof = rl.read_buffered(&mut con, &mut ans, &mut nul_warnings, &mut incomplete);
        if eof && incomplete.is_none() && !ans.is_empty() {
            ans.pop();
        }
        let lines: Vec<&str> = ans.iter().map(|s| s.as_deref().unwrap()).collect();
        assert_eq!(lines, vec!["foo", "", "", "foo"]);
    }
}
