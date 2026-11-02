//! Socket connections: `socketConnection`, `serverSocket`, `socketAccept`,
//! `socketSelect`, `socketTimeout` (GNU R's connections.c, and sockconn.c,
//! Rsock.c and sock.c of its internet module).

use super::io::{ConnKind, Connection, con_open_new, install_new};
use super::{arg, internal};
use crate::coerce::{as_integer, as_logical, as_real};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sys;
use crate::value::*;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};

pub static DEFS: &[super::Builtin] = &[
    internal("socketConnection", do_socket_connection, 8),
    internal("socketAccept", do_socket_accept, 6),
    internal("sockSelect", do_sock_select, 3),
    internal("serverSocket", do_server_socket, 1),
    internal("socketTimeout", do_socket_timeout, 2),
];

/// R: Rsock.c's `fd_set`s bound what a socket's descriptor can be.
const FD_LIMIT: i32 = sys::FD_SETSIZE;
const TOO_LARGE: &str = "file descriptor is too large for select()";

/// R: sockconn.c `struct sockconn`
pub struct SockConn {
    port: i32,
    server: bool,
    /// A handle on the listening socket, for a `socketAccept` connection
    /// created closed.
    serverfd: Option<OwnedFd>,
    timeout: i32,
    nodelay: bool,
    fd: Option<OwnedFd>,
    inbuf: Vec<u8>,
    pstart: usize,
    pend: usize,
}

impl SockConn {
    fn new(port: i32, server: bool, serverfd: Option<OwnedFd>, timeout: i32, nodelay: bool) -> SockConn {
        SockConn { port, server, serverfd, timeout, nodelay, fd: None, inbuf: vec![0; 4096], pstart: 0, pend: 0 }
    }

    /// R: Rsock.c:R_SockRead -- bytes read, 0 at the end of the stream or
    /// on a time out, else `-errno`.
    fn recv(&mut self, blocking: bool) -> isize {
        let Some(fd) = &self.fd else { return -(libc::EBADF as isize) };
        loop {
            if blocking {
                let w = sys::socket_wait(fd.as_fd(), false, self.timeout);
                if w != 0 {
                    return if w < 0 { w } else { 0 };
                }
            }
            match sys::socket_recv(fd.as_fd(), &mut self.inbuf) {
                Ok(n) => return n as isize,
                // spurious readability, can happen on Linux: `fd` is
                // non-blocking even for a blocking connection
                Err(libc::EWOULDBLOCK) if blocking => continue,
                Err(e) => return -(e as isize),
            }
        }
    }

    /// R: sockconn.c:sock_read_helper -- `(bytes copied, incomplete)`.
    pub fn read(&mut self, out: &mut [u8], blocking: bool) -> (usize, bool) {
        let mut nread = 0;
        while nread < out.len() {
            if self.pstart == self.pend {
                self.pstart = 0;
                self.pend = 0;
                let res = loop {
                    let res = self.recv(blocking);
                    if res != -(libc::EINTR as isize) {
                        break res;
                    }
                };
                if !blocking && res == -(libc::EWOULDBLOCK as isize) {
                    return (nread, true);
                }
                if res <= 0 {
                    return (nread, false);
                }
                self.pend = res as usize;
            }
            let n = (out.len() - nread).min(self.pend - self.pstart);
            out[nread..nread + n].copy_from_slice(&self.inbuf[self.pstart..self.pstart + n]);
            self.pstart += n;
            nread += n;
        }
        (nread, false)
    }

    /// R: Rsock.c:R_SockWrite -- whether everything was written.
    pub fn write(&mut self, mut bytes: &[u8]) -> bool {
        let Some(fd) = &self.fd else { return false };
        while !bytes.is_empty() {
            if sys::socket_wait(fd.as_fd(), true, self.timeout) != 0 {
                return false;
            }
            match sys::socket_send(fd.as_fd(), bytes) {
                Ok(n) => bytes = &bytes[n..],
                Err(libc::EWOULDBLOCK) => continue,
                Err(_) => return false,
            }
        }
        true
    }

    pub fn close(&mut self) {
        self.fd = None;
    }

    /// Opened for good, not by a read: closing destroys it now, and the
    /// handle would hold the server socket's port after `close(server)`.
    pub fn release_server(&mut self) {
        self.serverfd = None;
    }
}

/// R: sockconn.c:sock_open
pub(super) fn sock_open(i: &mut Interp, n: i32) -> RResult<bool> {
    let con = i.conns.get(n)?;
    let host = con.description.clone();
    let ConnKind::Sock(this) = &mut con.kind else { unreachable!() };
    let timeout = if this.timeout == NA_INTEGER || this.timeout <= 0 { 60 } else { this.timeout };
    this.pstart = 0;
    this.pend = 0;
    let (port, server, nodelay) = (this.port, this.server, this.nodelay);
    let serverfd = this.serverfd.take();
    let (fd, description) = if server {
        let (fd, name) = match serverfd {
            None => {
                let Some(sock1) = sys::socket_listen(port) else {
                    i.warning(format!("port {port} cannot be opened"))?;
                    return Ok(false);
                };
                if sock1.as_raw_fd() >= FD_LIMIT {
                    i.warning(TOO_LARGE)?;
                    return Ok(false);
                }
                let Some(accepted) = sys::socket_accept(sock1.as_fd(), timeout) else {
                    // NOTE: potentially confusing as the error was in accept()
                    i.warning("problem in listening on this socket")?;
                    return Ok(false);
                };
                accepted
            }
            Some(listening) => {
                let accepted = sys::socket_accept(listening.as_fd(), timeout);
                // each open accepts again: reading a closed connection opens it
                if let ConnKind::Sock(this) = &mut i.conns.get(n)?.kind {
                    this.serverfd = Some(listening);
                }
                match accepted {
                    Some(accepted) => accepted,
                    None => {
                        i.warning("problem in accepting connections on this socket")?;
                        return Ok(false);
                    }
                }
            }
        };
        let con = i.conns.get(n)?;
        if fd.as_raw_fd() >= FD_LIMIT && (con.canwrite || con.blocking) {
            i.warning(TOO_LARGE)?;
            return Ok(false);
        }
        (fd, format!("<-{name}:{port}"))
    } else {
        match sys::socket_connect(port, &host, timeout) {
            Some(fd) => (fd, format!("->{host}:{port}")),
            None => {
                i.warning(format!("{host}:{port} cannot be opened"))?;
                return Ok(false);
            }
        }
    };
    if nodelay {
        sys::socket_nodelay(fd.as_fd());
    }
    let con = i.conns.get(n)?;
    let ConnKind::Sock(this) = &mut con.kind else { unreachable!() };
    this.fd = Some(fd);
    con.description = description;
    con.isopen = true;
    con.text = !(con.mode.len() >= 2 && con.mode.ends_with('b'));
    con.reset_save();
    Ok(true)
}

fn str1(v: &Value, what: &str) -> RResult<String> {
    match v {
        Value::Str(s) if s.data.len() == 1 => Ok(s.data[0].as_deref().unwrap_or("NA").to_string()),
        _ => Err(Flow::error(format!("invalid '{what}' argument"))),
    }
}

/// `blocking, open, encoding, timeout, options`, as both entry points take.
// R: connections.c:do_sockconn
fn sock_common(
    i: &mut Interp,
    host: &str,
    (port, server, serverfd): (i32, bool, Option<OwnedFd>),
    args: &[Arg],
) -> RResult<Value> {
    let blocking = as_logical(&args[0].value);
    if blocking == NA_LOGICAL {
        return Err(Flow::error("invalid 'blocking' argument"));
    }
    let open = str1(&args[1].value, "open")?;
    let enc = str1(&args[2].value, "encoding")
        .and_then(|e| if e.len() > 100 { Err(Flow::error("invalid 'encoding' argument")) } else { Ok(e) })?;
    let timeout = as_integer(&args[3].value);
    // unknown options are no error, to allow for future extensions
    let nodelay = matches!(&args[4].value, Value::Str(o) if o.data.iter().any(|s| s.as_deref() == Some("no-delay")));
    let kind = ConnKind::Sock(SockConn::new(port, server, serverfd, timeout, nodelay));
    let mut con = Connection::new("sockconn", host, &open, kind);
    con.blocking = blocking != 0;
    con.encname = enc;
    let (v, n) = install_new(i, con)?;
    if !open.is_empty() {
        con_open_new(i, &v)?;
        if let ConnKind::Sock(this) = &mut i.conns.get(n)?.kind {
            this.release_server();
        }
    }
    Ok(v)
}

// R: connections.c:do_sockconn (PRIMVAL 0)
fn do_socket_connection(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let host = str1(&args[0].value, "host")?;
    let port = as_integer(&args[1].value);
    if port == NA_INTEGER || port < 0 {
        return Err(Flow::error("invalid 'port' argument"));
    }
    let server = as_logical(&args[2].value);
    if server == NA_LOGICAL {
        return Err(Flow::error("invalid 'server' argument"));
    }
    sock_common(i, &host, (port, server != 0, None), &args[3..])
}

// R: connections.c:do_sockconn (PRIMVAL 1), getConnectionCheck
fn do_socket_accept(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    if !arg(&args, 0).inherits("servsockconn") {
        return Err(Flow::error("'socket' is not a servsockconn"));
    }
    let (port, fd) = match &i.conns.get(as_integer(&args[0].value))?.kind {
        ConnKind::ServSock { port, fd } => (*port, fd.as_ref().and_then(|f| f.try_clone().ok())),
        _ => return Err(Flow::error("internal connection is not a servsockconn")),
    };
    if fd.is_none() {
        return Err(Flow::error("cannot open the connection"));
    }
    sock_common(i, "localhost", (port, true, fd), &args[1..])
}

// R: connections.c:do_serversocket, sockconn.c:in_R_newservsock
fn do_server_socket(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let port = as_integer(&arg(&args, 0));
    if port == NA_INTEGER || port < 0 {
        return Err(Flow::error("invalid 'port' argument"));
    }
    let Some(fd) = sys::socket_listen(port) else {
        return Err(Flow::error(format!("creation of server socket failed: port {port} cannot be opened")));
    };
    if fd.as_raw_fd() >= FD_LIMIT {
        return Err(Flow::error(TOO_LARGE));
    }
    let mut con = Connection::new("servsockconn", "localhost", "a+", ConnKind::ServSock { port, fd: Some(fd) });
    con.isopen = true;
    Ok(install_new(i, con)?.0)
}

// R: connections.c:do_sockselect
fn do_sock_select(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let Value::List(insock) = arg(&args, 0) else { return Err(Flow::error("not a list of sockets")) };
    if insock.data.is_empty() {
        return Err(Flow::error("not a list of sockets"));
    }
    let nsock = insock.data.len();
    let write = match arg(&args, 1) {
        Value::Lgl(w) if w.data.len() == nsock => w,
        _ => return Err(Flow::error("bad write indicators")),
    };
    let timeout = as_real(&arg(&args, 2));
    let mut fds = Vec::with_capacity(nsock);
    let mut val = vec![FALSE; nsock];
    let mut immediate = false;
    for (k, s) in insock.data.iter().enumerate() {
        let w = write.data[k] != 0;
        let (pfd, server) = match &i.conns.get(as_integer(s))?.kind {
            ConnKind::Sock(scp) => {
                if !w && scp.pstart < scp.pend {
                    val[k] = TRUE;
                    immediate = true;
                }
                (sys::pollfd(scp.fd.as_ref().map(|f| f.as_fd()), w), false)
            }
            ConnKind::ServSock { fd, .. } => (sys::pollfd(fd.as_ref().map(|f| f.as_fd()), w), true),
            _ => return Err(Flow::error("not a socket connection")),
        };
        if server && w {
            i.warning("a server socket connection cannot be writeable")?;
        }
        if pfd.fd >= FD_LIMIT && !immediate {
            return Err(Flow::error(TOO_LARGE));
        }
        fds.push(pfd);
    }
    if !immediate {
        // R: Rsock.c:R_SocketWaitMultiple
        if let Ok(true) = sys::socket_poll(&mut fds, timeout) {
            for (v, f) in val.iter_mut().zip(&fds) {
                *v = if f.revents != 0 { TRUE } else { FALSE };
            }
        }
    }
    Ok(Value::Lgl(std::rc::Rc::new(Vector::new(val))))
}

// R: connections.c:do_socktimeout
fn do_socket_timeout(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    if !arg(&args, 0).inherits("sockconn") {
        return Err(Flow::error("invalid 'socket' argument"));
    }
    let tnew = as_integer(&arg(&args, 1));
    let ConnKind::Sock(scon) = &mut i.conns.get(as_integer(&args[0].value))?.kind else {
        return Err(Flow::error("invalid 'socket' argument"));
    };
    let told = scon.timeout;
    if tnew == NA_INTEGER {
        return Err(Flow::error("invalid 'timeout' argument"));
    }
    if tnew >= 0 {
        scon.timeout = tnew;
    }
    Ok(Value::int(told))
}
