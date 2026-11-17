//! GNU R's public `struct Rconn` (`R_ext/Connections.h`, version 1, the A5
//! note's decision C): a connection's state as C reads and writes it through
//! `Rconnection` (`getConnection`, `R_GetConnection`). A robigo connection
//! holds one as its own state ([`crate::connections::Conn`]), so what C
//! writes there (`con->isopen`, `con->mode`, ...) is what R sees.
//!
//! The layout is GNU R's on x86-64 (LP64), measured against GNU R 4.6.2's
//! header and asserted here and in robigo's `Rconnections.h`
//! (`crates/robigo-rtapi/internal`). The method pointers are robigo-rtapi's
//! entries ([`CMethods`]), which call the runtime.

use std::ffi::{c_char, c_int, c_void};

/// `Rboolean` (an `int`-sized enum).
pub type Rboolean = c_int;

/// `struct Rconn`, GNU R's field order and types.
#[repr(C)]
pub struct RconnC {
    pub class: *mut c_char,
    pub description: *mut c_char,
    /// the encoding of `description`
    pub enc: c_int,
    pub mode: [c_char; 5],
    pub text: Rboolean,
    pub isopen: Rboolean,
    pub incomplete: Rboolean,
    pub canread: Rboolean,
    pub canwrite: Rboolean,
    pub canseek: Rboolean,
    pub blocking: Rboolean,
    pub is_gzcon: Rboolean,
    pub open: Option<unsafe extern "C" fn(*mut RconnC) -> Rboolean>,
    pub close: Option<unsafe extern "C" fn(*mut RconnC)>,
    pub destroy: Option<unsafe extern "C" fn(*mut RconnC)>,
    /// `int (*vfprintf)(struct Rconn *, const char *, va_list)`: a
    /// `va_list` is a pointer when passed (x86-64 SysV)
    pub vfprintf: Option<unsafe extern "C" fn(*mut RconnC, *const c_char, *mut c_void) -> c_int>,
    pub fgetc: Option<unsafe extern "C" fn(*mut RconnC) -> c_int>,
    pub fgetc_internal: Option<unsafe extern "C" fn(*mut RconnC) -> c_int>,
    pub seek: Option<unsafe extern "C" fn(*mut RconnC, f64, c_int, c_int) -> f64>,
    pub truncate: Option<unsafe extern "C" fn(*mut RconnC)>,
    pub fflush: Option<unsafe extern "C" fn(*mut RconnC) -> c_int>,
    pub read: Option<unsafe extern "C" fn(*mut c_void, usize, usize, *mut RconnC) -> usize>,
    pub write: Option<unsafe extern "C" fn(*const c_void, usize, usize, *mut RconnC) -> usize>,
    /// number of lines, position on top line
    pub n_push_back: c_int,
    pub pos_push_back: c_int,
    pub push_back: *mut *mut c_char,
    pub save: c_int,
    pub save2: c_int,
    pub encname: [c_char; 101],
    /// iconv_t, NULL if not in use
    pub inconv: *mut c_void,
    pub outconv: *mut c_void,
    pub iconvbuff: [c_char; 25],
    pub oconvbuff: [c_char; 50],
    pub next: *mut c_char,
    pub init_out: [c_char; 25],
    pub navail: i16,
    pub inavail: i16,
    pub eof_signalled: Rboolean,
    pub utf8out: Rboolean,
    pub id: *mut c_void,
    pub ex_ptr: *mut c_void,
    pub private: *mut c_void,
    /// for pipes etc
    pub status: c_int,
    pub buff: *mut u8,
    pub buff_len: usize,
    pub buff_stored_len: usize,
    pub buff_pos: usize,
}

// GNU R's layout (measured with GNU R 4.6.2's R_ext/Connections.h; the C
// side asserts the same numbers in robigo's Rconnections.h)
const _: () = {
    use std::mem::{offset_of, size_of};
    assert!(size_of::<RconnC>() == 488);
    assert!(offset_of!(RconnC, class) == 0);
    assert!(offset_of!(RconnC, description) == 8);
    assert!(offset_of!(RconnC, enc) == 16);
    assert!(offset_of!(RconnC, mode) == 20);
    assert!(offset_of!(RconnC, text) == 28);
    assert!(offset_of!(RconnC, isopen) == 32);
    assert!(offset_of!(RconnC, incomplete) == 36);
    assert!(offset_of!(RconnC, canread) == 40);
    assert!(offset_of!(RconnC, canwrite) == 44);
    assert!(offset_of!(RconnC, canseek) == 48);
    assert!(offset_of!(RconnC, blocking) == 52);
    assert!(offset_of!(RconnC, is_gzcon) == 56);
    assert!(offset_of!(RconnC, open) == 64);
    assert!(offset_of!(RconnC, close) == 72);
    assert!(offset_of!(RconnC, destroy) == 80);
    assert!(offset_of!(RconnC, vfprintf) == 88);
    assert!(offset_of!(RconnC, fgetc) == 96);
    assert!(offset_of!(RconnC, fgetc_internal) == 104);
    assert!(offset_of!(RconnC, seek) == 112);
    assert!(offset_of!(RconnC, truncate) == 120);
    assert!(offset_of!(RconnC, fflush) == 128);
    assert!(offset_of!(RconnC, read) == 136);
    assert!(offset_of!(RconnC, write) == 144);
    assert!(offset_of!(RconnC, n_push_back) == 152);
    assert!(offset_of!(RconnC, pos_push_back) == 156);
    assert!(offset_of!(RconnC, push_back) == 160);
    assert!(offset_of!(RconnC, save) == 168);
    assert!(offset_of!(RconnC, save2) == 172);
    assert!(offset_of!(RconnC, encname) == 176);
    assert!(offset_of!(RconnC, inconv) == 280);
    assert!(offset_of!(RconnC, outconv) == 288);
    assert!(offset_of!(RconnC, iconvbuff) == 296);
    assert!(offset_of!(RconnC, oconvbuff) == 321);
    assert!(offset_of!(RconnC, next) == 376);
    assert!(offset_of!(RconnC, init_out) == 384);
    assert!(offset_of!(RconnC, navail) == 410);
    assert!(offset_of!(RconnC, inavail) == 412);
    assert!(offset_of!(RconnC, eof_signalled) == 416);
    assert!(offset_of!(RconnC, utf8out) == 420);
    assert!(offset_of!(RconnC, id) == 424);
    assert!(offset_of!(RconnC, ex_ptr) == 432);
    assert!(offset_of!(RconnC, private) == 440);
    assert!(offset_of!(RconnC, status) == 448);
    assert!(offset_of!(RconnC, buff) == 456);
    assert!(offset_of!(RconnC, buff_len) == 464);
    assert!(offset_of!(RconnC, buff_stored_len) == 472);
    assert!(offset_of!(RconnC, buff_pos) == 480);
};

/// The C entry points a connection's methods are (robigo-rtapi's entries,
/// which C calls as `con->open(con)`, ...), registered per isolate by the C
/// API's install ([`crate::connections::set_c_methods`]).
#[derive(Clone, Copy, Debug)]
pub struct CMethods {
    pub open: unsafe extern "C" fn(*mut RconnC) -> Rboolean,
    pub close: unsafe extern "C" fn(*mut RconnC),
    pub destroy: unsafe extern "C" fn(*mut RconnC),
    pub vfprintf: unsafe extern "C" fn(*mut RconnC, *const c_char, *mut c_void) -> c_int,
    pub fgetc: unsafe extern "C" fn(*mut RconnC) -> c_int,
    pub fgetc_internal: unsafe extern "C" fn(*mut RconnC) -> c_int,
    pub seek: unsafe extern "C" fn(*mut RconnC, f64, c_int, c_int) -> f64,
    pub truncate: unsafe extern "C" fn(*mut RconnC),
    pub fflush: unsafe extern "C" fn(*mut RconnC) -> c_int,
    pub read: unsafe extern "C" fn(*mut c_void, usize, usize, *mut RconnC) -> usize,
    pub write: unsafe extern "C" fn(*const c_void, usize, usize, *mut RconnC) -> usize,
}

/// `init_con`'s `save` and `save2`: nothing saved.
pub const NO_SAVE: c_int = -1000;

impl RconnC {
    /// `init_con(new, description, enc, mode)`'s fields: closed, can read
    /// and write in principle, text, nothing pushed back or saved, no
    /// conversion or buffer, status NA (`class`, `description`, `id` and
    /// the methods are set by the connection that holds it).
    // R: connections.c:init_con
    pub fn new(mode: &str) -> RconnC {
        let mut c = RconnC {
            class: std::ptr::null_mut(),
            description: std::ptr::null_mut(),
            enc: 0,
            mode: [0; 5],
            text: 1,
            isopen: 0,
            incomplete: 0,
            canread: 1,
            canwrite: 1,
            canseek: 0,
            blocking: 0,
            is_gzcon: 0,
            open: None,
            close: None,
            destroy: None,
            vfprintf: None,
            fgetc: None,
            fgetc_internal: None,
            seek: None,
            truncate: None,
            fflush: None,
            read: None,
            write: None,
            n_push_back: 0,
            pos_push_back: 0,
            push_back: std::ptr::null_mut(),
            save: NO_SAVE,
            save2: NO_SAVE,
            encname: [0; 101],
            inconv: std::ptr::null_mut(),
            outconv: std::ptr::null_mut(),
            iconvbuff: [0; 25],
            oconvbuff: [0; 50],
            next: std::ptr::null_mut(),
            init_out: [0; 25],
            navail: 0,
            inavail: 0,
            eof_signalled: 0,
            utf8out: 0,
            id: std::ptr::null_mut(),
            ex_ptr: std::ptr::null_mut(),
            private: std::ptr::null_mut(),
            status: robigo_heap::NA_INTEGER,
            buff: std::ptr::null_mut(),
            buff_len: 0,
            buff_stored_len: 0,
            buff_pos: 0,
        };
        c.set_mode(mode);
        c
    }

    /// `mode`, at most 4 characters (`strncpy(new->mode, mode, 4)`).
    pub fn mode(&self) -> String {
        let n = self.mode.iter().position(|&b| b == 0).unwrap_or(self.mode.len());
        self.mode[..n].iter().map(|&b| b as u8 as char).collect()
    }

    pub fn set_mode(&mut self, m: &str) {
        self.mode = [0; 5];
        for (d, s) in self.mode.iter_mut().zip(m.bytes().take(4)) {
            *d = s as c_char;
        }
    }

    /// `encname`, at most 100 bytes.
    pub fn encname(&self) -> String {
        let n = self.encname.iter().position(|&b| b == 0).unwrap_or(self.encname.len());
        String::from_utf8_lossy(&self.encname[..n].iter().map(|&b| b as u8).collect::<Vec<u8>>()).into_owned()
    }

    pub fn set_encname(&mut self, e: &str) {
        self.encname = [0; 101];
        for (d, s) in self.encname.iter_mut().zip(e.bytes().take(100)) {
            *d = s as c_char;
        }
    }

    /// Point the method fields at `m`.
    pub fn set_methods(&mut self, m: &CMethods) {
        self.open = Some(m.open);
        self.close = Some(m.close);
        self.destroy = Some(m.destroy);
        self.vfprintf = Some(m.vfprintf);
        self.fgetc = Some(m.fgetc);
        self.fgetc_internal = Some(m.fgetc_internal);
        self.seek = Some(m.seek);
        self.truncate = Some(m.truncate);
        self.fflush = Some(m.fflush);
        self.read = Some(m.read);
        self.write = Some(m.write);
    }
}
