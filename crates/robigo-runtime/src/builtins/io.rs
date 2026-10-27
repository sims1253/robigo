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
use crate::format::{encode_integer, encode_logical, encode_real0, format_real};
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
];

const NCONNECTIONS: usize = 128;
const NSINKS: usize = 21;
/// `R_EOF`
const EOF: i32 = -1;

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
        temp: bool,
    },
    /// `textConnection(object, "r")`: the lines joined with newlines.
    TextIn {
        data: Vec<u8>,
        cur: usize,
    },
    /// `textConnection(name, "w")`: lines go to a character vector bound
    /// to `namesym` in `env` (or kept internally for `NULL`).
    TextOut {
        namesym: Option<Sym>,
        env: Env,
        data: Vec<RStr>,
        lastline: Vec<u8>,
    },
    /// `gzfile()`: read by decompressing the whole file when opened
    /// (uncompressed files are read as they are), written by compressing
    /// the buffered output when closed.
    Gz {
        level: u32,
        data: Vec<u8>,
        cur: usize,
        out: Vec<u8>,
        writing: bool,
        append: bool,
    },
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
fn con_open(i: &mut Interp, n: i32) -> RResult<bool> {
    let con = i.conns.get(n)?;
    let mode = con.mode.clone();
    let description = con.description.clone();
    if matches!(con.kind, ConnKind::Gz { .. }) {
        return gz_open(i, n);
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
    if let ConnKind::File { fp, rbuf, rpos, temp, .. } = &mut con.kind {
        *fp = Some(f);
        rbuf.clear();
        *rpos = 0;
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
    Ok(true)
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
            if flate2::read::MultiGzDecoder::new(&raw[..]).read_to_end(&mut data).is_err() {
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
    match &mut con.kind {
        ConnKind::Gz { data, cur, .. } | ConnKind::TextIn { data, cur } => {
            out.extend_from_slice(&data[*cur..]);
            *cur = data.len();
        }
        ConnKind::File { fp: Some(f), rbuf, rpos, .. } => {
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
    let con = i.conns.get(n)?;
    match &mut con.kind {
        ConnKind::Gz { out, writing: true, .. } => {
            out.extend_from_slice(bytes);
            Ok(())
        }
        ConnKind::File { fp: Some(f), .. } => f
            .write_all(bytes)
            .map_err(|e| Flow::error(format!("Error writing to connection:  {}", os_error_message(&e)))),
        _ => Err(Flow::error("cannot write to this connection")),
    }
}

fn i_tempdir() -> RResult<String> {
    super::sysutils::session_tempdir()
}

/// `con->close(con)`
fn con_close_raw(i: &mut Interp, n: i32) -> RResult<()> {
    let con = i.conns.get(n)?;
    match &mut con.kind {
        ConnKind::File { fp, .. } => {
            if let Some(mut f) = fp.take() {
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
        ConnKind::TextIn { .. } | ConnKind::Terminal => {}
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
    let ConnKind::File { fp: Some(f), rbuf, rpos, .. } = &mut con.kind else { return EOF };
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
fn rconn_fgetc(con: &mut Connection) -> i32 {
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
    match &mut con.kind {
        ConnKind::Terminal | ConnKind::TextIn { .. } => Err(Flow::error("cannot write to this connection")),
        ConnKind::Gz { out, writing, .. } => {
            if !*writing {
                return Err(Flow::error("cannot write to this connection"));
            }
            out.extend_from_slice(text.as_bytes());
            Ok(())
        }
        ConnKind::File { fp, .. } => match fp {
            Some(f) => {
                if let Err(e) = f.write_all(text.as_bytes()) {
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
    }
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
            if let ConnKind::File { fp: Some(f), .. } = &mut con.kind {
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
    let kind = ConnKind::File { raw, fp: None, rbuf: Vec::new(), rpos: 0, temp: false };
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
    }
    let nnn = if n < 0 { i64::MAX } else { n };
    let mut ans: Vec<RStr> = Vec::new();
    let mut nul_warnings: Vec<usize> = Vec::new();
    let mut incomplete: Option<Vec<u8>> = None;
    {
        let con = i.conns.get(ncon)?;
        con.incomplete = false;
        let mut nread: i64 = 0;
        while nread < nnn {
            let mut buf: Vec<u8> = Vec::new();
            let mut c;
            let mut saw_nul = false;
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
                if c == 0 {
                    saw_nul = true;
                }
                buf.push(c as u8);
            }
            if c == EOF {
                if !buf.is_empty() {
                    incomplete = Some(buf);
                }
                break;
            }
            let mut line: &[u8] = &buf;
            if saw_nul {
                if warn != 0 {
                    nul_warnings.push(nread as usize + 1);
                }
                let end = line.iter().position(|&b| b == 0).unwrap_or(line.len());
                line = &line[..end];
            }
            if nread == 0 && line.starts_with(b"\xef\xbb\xbf") {
                line = &line[3..];
            }
            ans.push(rstr(&String::from_utf8_lossy(line)));
            nread += 1;
        }
    }
    for k in nul_warnings {
        i.warning(format!("line {} appears to contain an embedded nul", k))?;
    }
    if !wasopen {
        con_close_raw(i, ncon)?;
    }
    if let Some(buf) = incomplete {
        let con = i.conns.get(ncon)?;
        if con.text && !con.blocking && con.class != "gzfile" {
            // push back the rest
            con.pushback.push(buf);
            con.pos_pushback = 0;
            con.incomplete = true;
        } else {
            let desc = con.description.clone();
            let mut line: &[u8] = &buf;
            if ans.is_empty() && line.starts_with(b"\xef\xbb\xbf") {
                line = &line[3..];
            }
            ans.push(rstr(&String::from_utf8_lossy(line)));
            if warn != 0 {
                i.warning(format!("incomplete final line found on '{}'", desc))?;
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
pub fn encode_element_cat(i: &Interp, v: &Value, k: usize) -> String {
    let pp = i.print_params();
    match v {
        Value::Lgl(x) => encode_logical(x.data[k], 0, "NA"),
        Value::Int(x) => encode_integer(x.data[k], 0, "NA"),
        Value::Real(x) => {
            let (w, d, e) = format_real(&x.data[k..k + 1], pp.digits, pp.scipen, 0, pp.na_width);
            encode_real0(x.data[k], w, d, e, "NA")
        }
        Value::Cplx(x) => {
            let z = x.data[k];
            let (r, im) = crate::format::format_complex(&[z], pp.digits, pp.scipen, 0, pp.na_width);
            crate::format::encode_complex(z, r, im, "NA")
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
                v if v.is_atomic() => Ok(encode_element_cat(i, v, k)),
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
