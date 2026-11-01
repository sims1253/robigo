//! Input/output: connections (GNU R's connections.c), `cat` (builtin.c
//! `do_cat`) and `readline` (scan.c).
//!
//! The connection table lives in `Interp::conns`. Connections 0, 1 and 2
//! are the terminals; `file()` and `textConnection()` connections take
//! the next free slot, as in R. Output written with `Rprintf` (all
//! printing) goes through `Interp::write_out`, which honours `sink()`.

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
    internal("writeBin", do_writebin, 5).custom_vis(),
    internal("readChar", do_readchar, 3),
    internal("writeChar", do_writechar, 5).custom_vis(),
];

const NCONNECTIONS: usize = 128;
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
    },
    /// `textConnection(object, "r")`: the lines joined with newlines.
    TextIn { data: Vec<u8>, cur: usize },
    /// `textConnection(name, "w")`: lines go to a character vector bound
    /// to `namesym` in `env` (or kept internally for `NULL`).
    TextOut { namesym: Option<Sym>, env: Env, data: Vec<RStr>, lastline: Vec<u8> },
    /// `gzfile()`: read by decompressing the whole file when opened
    /// (uncompressed files are read as they are), written by compressing
    /// the buffered output when closed.
    Gz { level: u32, data: Vec<u8>, cur: usize, out: Vec<u8>, writing: bool, append: bool },
    /// `rawConnection()`: an in-memory raw vector (R: connections.c
    /// rawconn). `pos` is shared for reading and writing; `nbytes` may be
    /// smaller than `data.len()` (the allocation is over-sized).
    Raw { data: Vec<u8>, pos: usize, nbytes: usize },
}

pub struct Connection {
    pub class: String,
    pub description: String,
    pub mode: String,
    pub text: bool,
    pub isopen: bool,
    pub incomplete: bool,
    pub canread: bool,
    pub canwrite: bool,
    pub canseek: bool,
    pub blocking: bool,
    pub encname: String,
    pub status: i32,
    /// CR/LF mapping state of `Rconn_fgetc` (`-1000`: nothing saved).
    save: i32,
    /// Pushed-back lines, last pushed first out.
    pushback: Vec<Vec<u8>>,
    pos_pushback: usize,
    pub kind: ConnKind,
    /// The `conn_id` external pointer.
    pub ex_ptr: Value,
}

impl Connection {
    /// R: connections.c:init_con
    fn new(class: &str, description: &str, mode: &str, kind: ConnKind) -> Connection {
        Connection {
            class: class.to_string(),
            description: description.to_string(),
            mode: mode.chars().take(4).collect(),
            text: true,
            isopen: false,
            incomplete: false,
            canread: true,
            canwrite: true,
            canseek: false,
            blocking: false,
            encname: String::new(),
            status: NA_INTEGER,
            save: -1000,
            pushback: Vec::new(),
            pos_pushback: 0,
            kind,
            ex_ptr: Value::Null,
        }
    }

    fn terminal(description: &str, mode: &str) -> Connection {
        let mut c = Connection::new("terminal", description, mode, ConnKind::Terminal);
        c.isopen = true;
        c.canread = mode == "r";
        c.canwrite = mode == "w";
        c
    }
}

const FILE_WBUF_SIZE: usize = 8192;

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
        if n < 0 || n as usize >= NCONNECTIONS || n == NA_INTEGER {
            return Err(Flow::error("invalid connection"));
        }
        match self.table[n as usize].as_deref_mut() {
            Some(c) => Ok(c),
            None => Err(Flow::error("invalid connection")),
        }
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

unsafe extern "C" {
    fn strerror(errnum: std::ffi::c_int) -> *const std::ffi::c_char;
}

/// The C library's message for an OS error, as R prints with
/// `strerror(errno)`.
pub fn os_error_message(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(code) => unsafe {
            let p = strerror(code);
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        },
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

/// Open connection `n` with its current mode (R's `con->open(con)`).
/// Failures are reported with a warning and `Ok(false)`.
pub(super) fn con_open(i: &mut Interp, n: i32) -> RResult<bool> {
    let con = i.conns.get(n)?;
    let mode = con.mode.clone();
    let description = con.description.clone();
    if matches!(con.kind, ConnKind::Gz { .. }) {
        return gz_open(i, n);
    }
    if matches!(con.kind, ConnKind::File { fifo: true, .. }) {
        return fifo_open(i, n);
    }
    if !matches!(con.kind, ConnKind::File { .. }) {
        con.save = -1000;
        return Ok(true);
    }
    // R: connections.c:file_open
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
    con.save = -1000;
    init_file_positions(con);
    set_iconv_check(i, n)?;
    Ok(true)
}

unsafe extern "C" {
    fn iconv_open(tocode: *const std::ffi::c_char, fromcode: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    fn iconv_close(cd: *mut std::ffi::c_void) -> std::ffi::c_int;
}

/// Whether glibc's iconv can convert `from` to `to` ("" is the locale
/// charset, as R's `Riconv_open` on Unix relies on).
fn iconv_converts(to: &str, from: &str) -> bool {
    let (Ok(t), Ok(f)) = (std::ffi::CString::new(to), std::ffi::CString::new(from)) else { return false };
    // SAFETY: both pointers are valid C strings for the call's duration.
    let cd = unsafe { iconv_open(t.as_ptr(), f.as_ptr()) };
    if cd as isize == -1 {
        return false;
    }
    // SAFETY: cd is a converter iconv_open returned.
    unsafe { iconv_close(cd) };
    true
}

/// `set_iconv`'s validity check (R: connections.c set_iconv): a text
/// connection with a non-native `encoding` must have a working conversion
/// in each direction it is open for, or the connection is destroyed and
/// an error is raised. robigo reads text as bytes, but the check must
/// match R's.
fn set_iconv_check(i: &mut Interp, n: i32) -> RResult<()> {
    let (text, encname, canread, canwrite) = {
        let con = i.conns.get(n)?;
        (con.text, con.encname.clone(), con.canread, con.canwrite)
    };
    if !text || encname.is_empty() || encname == "native.enc" {
        return Ok(());
    }
    // readLines()/scan() set UTF8out; at open it is always false, so the
    // target of the input conversion is "" (the locale charset).
    let enc = if encname == "UTF-8-BOM" { "UTF-8" } else { encname.as_str() };
    if canread && !iconv_converts("", enc) {
        i.conns.table[n as usize] = None;
        return Err(Flow::error(format!("unsupported conversion from '{}' to '{}'", encname, "")));
    }
    if canwrite && !iconv_converts(encname.as_str(), "") {
        i.conns.table[n as usize] = None;
        return Err(Flow::error(format!("unsupported conversion from '{}' to '{}'", encname, "")));
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

unsafe extern "C" {
    fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> std::ffi::c_int;
    fn open(path: *const std::ffi::c_char, flags: std::ffi::c_int, ...) -> std::ffi::c_int;
}

/// `fifo()`'s open: create the fifo when writing, then open it with
/// `O_NONBLOCK` unless the connection is blocking.
// R: connections.c fifo_open
fn fifo_open(i: &mut Interp, n: i32) -> RResult<bool> {
    use std::os::unix::io::FromRawFd;
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
    let c = match std::ffi::CString::new(name.as_bytes()) {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };
    // SAFETY: open takes the path and flags only.
    let fd = unsafe { open(c.as_ptr(), flags) };
    if fd < 0 {
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if err == ENXIO {
            i.warning(format!("fifo '{}' is not ready", name))?;
        } else {
            i.warning(format!("cannot open fifo '{}'", name))?;
        }
        return Ok(false);
    }
    if is_temp {
        let _ = std::fs::remove_file(&name);
    }
    // SAFETY: fd is an open file descriptor we now own.
    let f = unsafe { File::from_raw_fd(fd) };
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
    con.save = -1000;
    init_file_positions(con);
    set_iconv_check(i, n)?;
    Ok(true)
}

/// `mkfifo(name, 00644)`, returning errno on failure (0 on success).
fn c_open_mkfifo(name: &str) -> i32 {
    let Ok(c) = std::ffi::CString::new(name.as_bytes()) else { return 0 };
    // SAFETY: mkfifo takes the path string only.
    if unsafe { mkfifo(c.as_ptr(), 0o644) } == 0 {
        return 0;
    }
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Open a `gzfile` connection.
// R: connections.c:gzfile_open
fn gz_open(i: &mut Interp, n: i32) -> RResult<bool> {
    let con = i.conns.get(n)?;
    let name = expand_file_name(&con.description);
    let m = con.mode.clone();
    let writing = m.starts_with('w') || m.starts_with('a');
    let mut data = Vec::new();
    if !writing {
        let raw = match std::fs::read(&name) {
            Ok(d) => d,
            Err(e) => {
                i.warning(format!(
                    "cannot open compressed file '{}', probable reason '{}'",
                    name,
                    os_error_message(&e)
                ))?;
                return Ok(false);
            }
        };
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
    } else if let Err(e) = std::fs::OpenOptions::new().write(true).create(true).append(true).open(&name) {
        i.warning(format!("cannot open compressed file '{}', probable reason '{}'", name, os_error_message(&e)))?;
        return Ok(false);
    }
    let con = i.conns.get(n)?;
    if let ConnKind::Gz { data: d, cur, out, writing: w, append, .. } = &mut con.kind {
        *d = data;
        *cur = 0;
        out.clear();
        *w = writing;
        *append = m.starts_with('a');
    }
    con.isopen = true;
    con.canread = !writing;
    con.canwrite = writing;
    con.text = m.contains('t');
    con.save = -1000;
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
    Ok(rconn_fgetc(i.conns.get(n)?))
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

/// Open connection `n` in `mode` without changing its recorded mode, as
/// serialize.c does for connections it was given closed.
pub fn open_in_mode(i: &mut Interp, n: i32, mode: &str) -> RResult<bool> {
    let saved = std::mem::replace(&mut i.conns.get(n)?.mode, mode.to_string());
    let r = con_open(i, n);
    i.conns.get(n)?.mode = saved;
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
/// `con->read` until end of input).
pub fn con_read_all(i: &mut Interp, n: i32) -> RResult<Vec<u8>> {
    let con = i.conns.get(n)?;
    let mut out: Vec<u8> = Vec::new();
    while con.pos_pushback < con.pushback.len() {
        // Pushed-back lines come first.
        let line = con.pushback.pop().unwrap_or_default();
        out.extend_from_slice(&line);
    }
    file_switch_to_read(&mut con.kind);
    match &mut con.kind {
        ConnKind::Gz { data, cur, .. } | ConnKind::TextIn { data, cur } => {
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
        ConnKind::Terminal => {
            let _ = std::io::stdin().read_to_end(&mut out);
        }
        _ => return Err(Flow::error("cannot read from this connection")),
    }
    Ok(out)
}

/// Write bytes to a connection (R's `con->write`).
pub fn con_write_bytes(i: &mut Interp, n: i32, bytes: &[u8]) -> RResult<()> {
    // the terminals and text connections take text (Rconn_printf from C
    // through con->vfprintf, as R's write.table does to stdout())
    let byte_kind = matches!(
        i.conns.get(n)?.kind,
        ConnKind::Gz { writing: true, .. } | ConnKind::Raw { .. } | ConnKind::File { fp: Some(_), .. }
    );
    if n <= 2 || !byte_kind {
        return conn_write(i, n, &String::from_utf8_lossy(bytes));
    }
    let con = i.conns.get(n)?;
    file_switch_to_write(&mut con.kind);
    match &mut con.kind {
        ConnKind::Gz { out, writing: true, .. } => {
            out.extend_from_slice(bytes);
            Ok(())
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
            Ok(())
        }
        ConnKind::File { fp: Some(f), wbuf, fifo, last_was_write, .. } => {
            *last_was_write = true;
            file_write_buffered(f, wbuf, *fifo, bytes)
                .map_err(|e| Flow::error(format!("Error writing to connection:  {}", os_error_message(&e))))
        }
        _ => Err(Flow::error("cannot write to this connection")),
    }
}

fn i_tempdir() -> RResult<String> {
    super::sysutils::session_tempdir()
}

/// `con->close(con)`
pub(super) fn con_close_raw(i: &mut Interp, n: i32) -> RResult<()> {
    let con = i.conns.get(n)?;
    match &mut con.kind {
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
                        let l = rstr(&String::from_utf8_lossy(lastline));
                        data.push(l);
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
        ConnKind::Gz { level, out, writing, append, data, .. } => {
            if *writing {
                let name = expand_file_name(&con.description);
                let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(*level));
                let ok = enc.write_all(out).is_ok();
                let bytes = enc.finish().ok().filter(|_| ok);
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
            con.isopen = false;
        }
        ConnKind::TextIn { .. } | ConnKind::Terminal | ConnKind::Raw { .. } => {}
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
fn con_destroy(i: &mut Interp, n: i32) -> RResult<i32> {
    check_close(i, n)?;
    let st = i.conns.get(n)?.status;
    i.conns.table[n as usize] = None;
    Ok(st)
}

fn file_fgetc(con: &mut Connection) -> i32 {
    let ConnKind::File { fp: Some(f), rbuf, rpos, last_was_write, fifo, wbuf, frpos, wpos, .. } = &mut con.kind else {
        return EOF;
    };
    if *last_was_write {
        if !*fifo {
            use std::io::{Seek, SeekFrom};
            let _ = flush_file_wbuf(f, wbuf);
            *wpos = f.stream_position().unwrap_or(*wpos);
            let _ = f.seek(SeekFrom::Start(*frpos));
            rbuf.clear();
            *rpos = 0;
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
        rbuf.resize(8192, 0);
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

/// `con->fgetc(con)` without CR mapping or pushback.
fn con_fgetc_internal(con: &mut Connection) -> i32 {
    match &mut con.kind {
        ConnKind::TextIn { data, cur } => {
            if *cur >= data.len() {
                EOF
            } else {
                *cur += 1;
                data[*cur - 1] as i32
            }
        }
        ConnKind::File { .. } => file_fgetc(con),
        ConnKind::Gz { data, cur, .. } => {
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
        ConnKind::Terminal => {
            let mut b = [0u8; 1];
            match std::io::stdin().read(&mut b) {
                Ok(1) => b[0] as i32,
                _ => EOF,
            }
        }
        ConnKind::TextOut { .. } => EOF,
    }
}

/// R: connections.c:Rconn_fgetc
pub(super) fn rconn_fgetc(con: &mut Connection) -> i32 {
    if con.pushback.is_empty() {
        if con.save != -1000 {
            let c = con.save;
            con.save = -1000;
            return c;
        }
        let c = con_fgetc_internal(con);
        if c == '\r' as i32 {
            let c2 = con_fgetc_internal(con);
            if c2 != '\n' as i32 {
                con.save = if c2 != '\r' as i32 { c2 } else { '\n' as i32 };
                return '\n' as i32;
            }
            return c2;
        }
        return c;
    }
    let last = con.pushback.len() - 1;
    let c = con.pushback[last][con.pos_pushback] as i32;
    con.pos_pushback += 1;
    if con.pos_pushback >= con.pushback[last].len() {
        con.pushback.pop();
        con.pos_pushback = 0;
    }
    c
}

/// Write text to connection `n` (R's `Rconn_printf(con, "%s", s)`); the
/// terminals write to the process streams directly.
pub fn conn_write(i: &mut Interp, n: i32, text: &str) -> RResult<()> {
    match n {
        1 => {
            let _ = i.out.write_all(text.as_bytes());
            return Ok(());
        }
        2 => {
            let _ = i.out.flush();
            let _ = i.err.write_all(text.as_bytes());
            return Ok(());
        }
        _ => {}
    }
    let con = i.conns.get(n)?;
    let mut raw_bytes: Option<Vec<u8>> = None;
    file_switch_to_write(&mut con.kind);
    match &mut con.kind {
        ConnKind::Terminal | ConnKind::TextIn { .. } => return Err(Flow::error("cannot write to this connection")),
        ConnKind::Gz { out, writing, .. } => {
            if !*writing {
                return Err(Flow::error("cannot write to this connection"));
            }
            out.extend_from_slice(text.as_bytes());
            return Ok(());
        }
        // Rconn_printf on a raw connection formats then calls con->write
        // (connections.c dummy_vfprintf).
        ConnKind::Raw { .. } => {
            raw_bytes = Some(text.as_bytes().to_vec());
            Ok(())
        }
        ConnKind::File { fp, wbuf, fifo, last_was_write, .. } => match fp {
            Some(f) => {
                *last_was_write = true;
                if let Err(e) = file_write_buffered(f, wbuf, *fifo, text.as_bytes()) {
                    return Err(Flow::error(format!("Error writing to connection:  {}", os_error_message(&e))));
                }
                Ok(())
            }
            None => Err(Flow::error("cannot write to this connection")),
        },
        ConnKind::TextOut { .. } => {
            // R: connections.c:text_vfprintf
            let ConnKind::TextOut { namesym, env, data, lastline } = &mut con.kind else { unreachable!() };
            lastline.extend_from_slice(text.as_bytes());
            let mut changed = false;
            while let Some(pos) = lastline.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = lastline.drain(..=pos).collect();
                data.push(rstr(&String::from_utf8_lossy(&line[..line.len() - 1])));
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

fn conn_flush(i: &mut Interp, n: i32) -> RResult<()> {
    match n {
        1 => {
            let _ = i.out.flush();
        }
        2 => {
            let _ = i.err.flush();
        }
        _ => {
            let con = i.conns.get(n)?;
            if let ConnKind::File { fp: Some(f), wbuf, .. } = &mut con.kind {
                let _ = flush_file_wbuf(f, wbuf);
                let _ = f.flush();
            }
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

/// `REprintf` while the message connection is diverted.
pub fn reprintf_sink(i: &mut Interp, text: &str) {
    let n = i.conns.error_con;
    if conn_write(i, n, text).is_err() {
        i.conns.error_con = 2;
        let _ = i.err.write_all(text.as_bytes());
    }
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
    if n > 2 && !con.ex_ptr.is_null() {
        v.set_attr_raw(Sym::new("conn_id"), con.ex_ptr.clone());
    }
    v
}

fn new_ex_ptr() -> Value {
    Value::ExtPtr(Rc::new(ExtPtr { tag: Value::sym("connection"), attrs: Default::default(), payload: None }))
}

/// Register a new connection in slot `n` and return its R object.
fn install_connection(i: &mut Interp, n: usize, mut con: Connection) -> Value {
    con.ex_ptr = new_ex_ptr();
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

/// R's `asVecSize`; `-999` signals an invalid value.
pub fn as_vec_size(v: &Value) -> RResult<i64> {
    if v.is_atomic() && !v.is_empty() {
        match v {
            Value::Int(x) => {
                if x.data[0] == NA_INTEGER {
                    return Err(Flow::error("vector size cannot be NA"));
                }
                return Ok(x.data[0] as i64);
            }
            Value::Real(x) => {
                let d = x.data[0];
                if d.is_nan() {
                    return Err(Flow::error("vector size cannot be NA/NaN"));
                }
                if !d.is_finite() {
                    return Err(Flow::error("vector size cannot be infinite"));
                }
                if d > 4503599627370496.0 {
                    return Err(Flow::error("vector size specified is too large"));
                }
                return Ok(d as i64);
            }
            Value::Str(x) => {
                let d = x.data[0].as_deref().and_then(crate::coerce::str_to_real).unwrap_or(NA_REAL);
                if d.is_nan() {
                    return Err(Flow::error("vector size cannot be NA/NaN"));
                }
                if !d.is_finite() {
                    return Err(Flow::error("vector size cannot be infinite"));
                }
                if d > 4503599627370496.0 {
                    return Err(Flow::error("vector size specified is too large"));
                }
                return Ok(d as i64);
            }
            _ => {}
        }
    }
    Ok(-999)
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

unsafe extern "C" {
    fn isatty(fd: std::ffi::c_int) -> std::ffi::c_int;
}

fn do_isatty(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let con = as_integer(&arg(&args, 0));
    let r = if con == NA_INTEGER || !(0..=2).contains(&con) { false } else { unsafe { isatty(con) != 0 } };
    Ok(Value::lgl(r))
}

/// Check for a compressed file (R: connections.c:comp_type_from_file).
fn is_compressed_file(path: &str) -> bool {
    let mut buf = [0u8; 7];
    let Ok(mut f) = File::open(path) else { return false };
    let n = f.read(&mut buf).unwrap_or(0);
    if n >= 2 && buf[0] == 0x1f && buf[1] == 0x8b {
        return true;
    }
    if n >= 3 && &buf[..3] == b"BZh" {
        return true;
    }
    if n >= 5 && buf[0] == 0xFD && &buf[1..5] == b"7zXZ" {
        return true;
    }
    if n >= 4 && buf[..4] == [0x28, 0xB5, 0x2F, 0xFD] {
        return true;
    }
    false
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
    let ncon = i.conns.next_connection()?;
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
        if !raw && (open.is_empty() || open == "r" || open == "rt") && is_compressed_file(&efn) {
            return Err(Flow::error("robigo: compressed file connections are not implemented yet"));
        }
    }
    let mode = if open.is_empty() { "r".to_string() } else { open.clone() };
    let kind = ConnKind::File {
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
    };
    let mut con = Connection::new("file", &description, &mode, kind);
    con.canseek = !raw;
    con.blocking = block != 0;
    con.encname = enc.chars().take(99).collect();
    if !con.encname.is_empty() && con.encname != "native.enc" {
        con.canseek = false;
    }
    let v = install_connection(i, ncon, con);
    if !open.is_empty() {
        // R: connections.c:checked_open
        let ok = match con_open(i, ncon as i32) {
            Ok(ok) => ok,
            Err(e) => {
                i.conns.table[ncon] = None;
                return Err(e);
            }
        };
        if !ok {
            i.conns.table[ncon] = None;
            return Err(Flow::error("cannot open the connection"));
        }
    }
    Ok(v)
}

fn do_url(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Err(Flow::error("robigo: url() connections are not implemented yet"))
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
    let ncon = i.conns.next_connection()?;
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
    };
    let mut con = Connection::new("fifo", &sfile, &mode, kind);
    con.blocking = block;
    con.encname = enc;
    let v = install_connection(i, ncon, con);
    if !open.is_empty() {
        // R: connections.c:checked_open
        let ok = match con_open(i, ncon as i32) {
            Ok(ok) => ok,
            Err(e) => {
                i.conns.table[ncon] = None;
                return Err(e);
            }
        };
        if !ok {
            i.conns.table[ncon] = None;
            return Err(Flow::error("cannot open the connection"));
        }
    }
    Ok(v)
}

/// `gzfile()`; `bzfile()`, `xzfile()` and `zstdfile()` share the entry
/// point (R's PRIMVAL), but only gzip is implemented.
// R: connections.c:do_gzfile
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
    if ty == 0 && (open.is_empty() || open.starts_with('r')) {
        // check magic no
        let efn = expand_file_name(&file);
        let mut buf = [0u8; 6];
        let k = File::open(&efn).and_then(|mut f| f.read(&mut buf)).unwrap_or(0);
        if k >= 3 && &buf[..3] == b"BZh" {
            ty = 1;
        } else if k >= 5 && buf[0] == 0xFD && &buf[1..5] == b"7zXZ" {
            ty = 2;
        } else if k >= 4 && buf[..4] == [0x28, 0xB5, 0x2F, 0xFD] {
            ty = 3;
        }
    }
    if ty != 0 {
        let what = ["", "bzip2", "xz", "zstd"][ty];
        return Err(Flow::error(format!("robigo: {} compressed file connections are not implemented yet", what)));
    }
    let ncon = i.conns.next_connection()?;
    let mode = if open.is_empty() { "rb".to_string() } else { open.clone() };
    let kind = ConnKind::Gz {
        level: compress as u32,
        data: Vec::new(),
        cur: 0,
        out: Vec::new(),
        writing: false,
        append: false,
    };
    let mut con = Connection::new("gzfile", &file, &mode, kind);
    con.blocking = true;
    con.canseek = false;
    con.encname = enc;
    let v = install_connection(i, ncon, con);
    if !open.is_empty() {
        // R: connections.c:checked_open
        let ok = match con_open(i, ncon as i32) {
            Ok(ok) => ok,
            Err(e) => {
                i.conns.table[ncon] = None;
                return Err(e);
            }
        };
        if !ok {
            i.conns.table[ncon] = None;
            return Err(Flow::error("cannot open the connection"));
        }
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
    let ncon = i.conns.next_connection()?;
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
    let n = as_vec_size(&arg(&args, 1))?;
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
    match arg(&args, 4) {
        Value::Str(s) if s.data.len() == 1 => {}
        _ => return Err(Flow::error("invalid 'encoding' value")),
    }
    let skip_nul = as_logical(&arg(&args, 5));
    if skip_nul == NA_LOGICAL {
        return Err(Flow::error("invalid 'skipNul' argument"));
    }
    let wasopen = i.conns.get(ncon)?.isopen;
    if !wasopen {
        let con = i.conns.get(ncon)?;
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
    let nnn = if n < 0 { i64::MAX } else { n };
    let mut ans: Vec<RStr> = Vec::new();
    let mut nul_warnings: Vec<usize> = Vec::new();
    // R: the incomplete final line is stored like any other before
    // `no_more_lines` handling; `incomplete_buf` holds it for the
    // pushback-or-warn decision.
    let mut incomplete_buf: Option<Vec<u8>> = None;
    let mut eof = false;
    {
        let con = i.conns.get(ncon)?;
        con.incomplete = false;
        let mut nread: i64 = 0;
        while nread < nnn && !eof {
            let mut buf: Vec<u8> = Vec::new();
            let mut c;
            loop {
                c = rconn_fgetc(con);
                if c == EOF {
                    break;
                }
                if skip_nul != 0 && c == 0 {
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
            ans.push(rstr(&String::from_utf8_lossy(line)));
            if nul && warn != 0 {
                nul_warnings.push(nread as usize + 1);
            }
            nread += 1;
            if c == EOF {
                eof = true;
                if !buf.is_empty() {
                    incomplete_buf = Some(buf);
                }
            }
        }
    }
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

/// `writeLines(text, con, sep, useBytes)`
fn do_writelines(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let text = match arg(&args, 0) {
        Value::Str(t) => t.data.clone(),
        _ => return Err(Flow::error("invalid 'text' argument")),
    };
    let mut con_num = con_arg(&arg(&args, 1))?;
    i.conns.get(con_num)?;
    let sep = match arg(&args, 2) {
        Value::Str(s) => s.data.first().cloned().flatten().map(|s| s.to_string()).unwrap_or_else(|| "NA".into()),
        _ => return Err(Flow::error("invalid 'sep' argument")),
    };
    if as_logical(&arg(&args, 3)) == NA_LOGICAL {
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
    let mut out = String::new();
    for t in &text {
        out.push_str(t.as_deref().unwrap_or("NA"));
        out.push_str(&sep);
    }
    if con_num == i.conns.output_con {
        let mut j = 0;
        loop {
            conn_write(i, con_num, &out)?;
            conn_flush(i, con_num)?;
            con_num = i.conns.active_sink(j);
            j += 1;
            if con_num <= 0 {
                break;
            }
        }
    } else {
        conn_write(i, con_num, &out)?;
    }
    if !wasopen {
        check_close(i, con_num)?;
    }
    Ok(Value::Null)
}

/// R: scan.c:do_readln (non-interactive: simulate a CR response).
fn do_readln(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let prompt = arg(&args, 0);
    let p = if prompt.is_null() {
        String::new()
    } else {
        let pv = i.coerce(&prompt, VType::Str)?;
        match &pv {
            Value::Str(s) if !s.data.is_empty() => s.data[0].as_deref().unwrap_or("NA").to_string(),
            _ => String::new(),
        }
    };
    i.write_out(&format!("{}\n", p));
    Ok(Value::str(""))
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
    let ncon = i.conns.next_connection()?;
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
    let r#where = pos_arg;
    use std::io::Seek;
    let con = i.conns.get(n)?;
    // R: connections.c null_seek — fifos cannot seek.
    if matches!(con.kind, ConnKind::File { fifo: true, .. }) {
        return Err(Flow::error("'seek' not enabled for this connection"));
    }
    if matches!(con.kind, ConnKind::File { .. }) {
        // R: connections.c file_seek (the read buffer holds bytes already
        // consumed from the OS file)
        let (canread, canwrite) = (con.canread, con.canwrite);
        let ConnKind::File { fp: Some(f), rbuf, rpos: rb, wbuf, last_was_write, frpos, wpos, .. } = &mut con.kind
        else {
            return Ok(0.0);
        };
        let _ = flush_file_wbuf(f, wbuf);
        // make sure both positions are set
        let os_pos = f.stream_position().unwrap_or(0);
        if *last_was_write {
            *wpos = os_pos;
        } else {
            *frpos = os_pos - (rbuf.len() - *rb) as u64;
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
        // SEEK_CUR is relative to the logical position, not the buffer's end
        if let std::io::SeekFrom::Current(_) = whence {
            let logical = os_pos - (rbuf.len() - *rb) as u64;
            let _ = f.seek(std::io::SeekFrom::Start(logical));
        }
        rbuf.clear();
        *rb = 0;
        let new = f.seek(whence).unwrap_or(0);
        if *last_was_write {
            *wpos = new;
        } else {
            *frpos = new;
        }
        return Ok(pos as f64);
    }
    match &mut con.kind {
        // R: connections.c raw_seek
        ConnKind::Raw { pos, nbytes, .. } => {
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
        _ => Err(Flow::error("seek is not relevant for text connection")),
    }
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
        _ => return Err(Flow::error("truncation not enabled for this connection")),
    }
    Ok(Value::Null)
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
    if used == buf.len() {
        return Ok(used);
    }
    file_switch_to_read(&mut con.kind);
    match &mut con.kind {
        ConnKind::TextIn { data, cur } | ConnKind::Gz { data, cur, .. } => {
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
        ConnKind::Terminal => match std::io::stdin().read(&mut buf[used..]) {
            Ok(k) => Ok(used + k),
            Err(_) => Err(Flow::error("error reading from the connection")),
        },
        _ => Err(Flow::error("cannot read from this connection")),
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
    let n = as_vec_size(&arg(&args, 2))?;
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
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    let mut b = [0u8; 16];
                    let got = readbin_read(i, ncon, bytes.as_ref(), &mut np, &mut b)?;
                    if got < 16 {
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
                let mut ints: Vec<i32> = Vec::with_capacity(n);
                for _ in 0..n {
                    let mut b = [0u8; 8];
                    let got = readbin_read(i, ncon, bytes.as_ref(), &mut np, &mut b[..size as usize])?;
                    if got < size as usize {
                        break;
                    }
                    let v = read_int_bytes(&b, size, signd, swap);
                    ints.push(v);
                }
                Ok(if is_logical {
                    Value::lgls(ints)
                } else if is_raw {
                    Value::raws(ints.into_iter().map(|x| x as u8).collect())
                } else {
                    Value::ints(ints)
                })
            }
            w if w == "numeric" || w == "double" => {
                if size == NA_INTEGER {
                    size = 8;
                }
                match size {
                    4 | 8 | 16 => {}
                    _ => return Err(Flow::error(format!("size {} is unknown on this machine", size))),
                }
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    let mut b = [0u8; 16];
                    let got = readbin_read(i, ncon, bytes.as_ref(), &mut np, &mut b[..size as usize])?;
                    if got < size as usize {
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
                    4 | 8 => {}
                    _ => return Err(Flow::error(format!("size {} is unknown on this machine", size))),
                }
                let mut out = vec![0u8; len * size as usize];
                for (k, &v) in x.data.iter().enumerate() {
                    let b = if size == 4 { (v as f32).to_le_bytes().to_vec() } else { v.to_le_bytes().to_vec() };
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
        Value::Str(v) => v.data.iter().map(|x| x.as_deref().unwrap_or("NA").to_string()).collect(),
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
        Value::Str(v) => v.data.iter().map(|x| x.as_deref().unwrap_or("NA").to_string()).collect(),
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
    let sep_at = |k: usize| -> &str { if sepr.is_empty() { "" } else { &sepr[k % sepr.len()] } };
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
    // One PrintParams for the whole call: `encode_element_cat` used to build
    // it per element, costing five option lookups per cat() element.
    let pp = i.print_params();
    for (iobj, s0) in objs.iter().enumerate() {
        if iobj != 0 && !s0.is_null() {
            out.push_str(sep_at(ntot));
            ntot += 1;
        }
        let n = s0.len();
        if n == 0 {
            continue;
        }
        if !labs.is_empty() && iobj == 0 && as_integer(fill) > 0 {
            let l = &labs[nlines % labs.len()];
            out.push_str(l);
            out.push(' ');
            width += crate::format::str_width(l) + 1;
            nlines += 1;
        }
        let elem = |k: usize| -> RResult<String> {
            match s0 {
                Value::Sym(sy) => Ok(sy.name().to_string()),
                v if v.is_atomic() => Ok(encode_element_cat(&pp, v, k)),
                v => Err(Flow::error(format!(
                    "argument {} (type '{}') cannot be handled by 'cat'",
                    iobj + 1,
                    v.type_name()
                ))),
            }
        };
        let mut p = elem(0)?;
        let mut w = p.len();
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
                w = p.len();
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
        if let ConnKind::File { fp: Some(f), rbuf, rpos, wbuf, last_was_write, frpos, wpos, fifo: false, .. } = kind {
            use std::io::{Seek, SeekFrom};
            let _ = flush_file_wbuf(f, wbuf);
            *wpos = f.stream_position().unwrap_or(*wpos);
            *last_was_write = false;
            let _ = f.seek(SeekFrom::Start(*frpos));
            rbuf.clear();
            *rpos = 0;
        }
    }
}

/// R: connections.c:file_write — before writing after a read: remember
/// the read position (less what is buffered), go to the write position.
#[inline]
fn file_switch_to_write(kind: &mut ConnKind) {
    if let ConnKind::File { last_was_write: false, .. } = kind {
        if let ConnKind::File { fp: Some(f), rbuf, rpos, last_was_write, frpos, wpos, fifo: false, .. } = kind {
            use std::io::{Seek, SeekFrom};
            let buffered = (rbuf.len() - *rpos) as u64;
            *frpos = f.stream_position().unwrap_or(0).saturating_sub(buffered);
            *last_was_write = true;
            let _ = f.seek(SeekFrom::Start(*wpos));
            rbuf.clear();
            *rpos = 0;
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
}
