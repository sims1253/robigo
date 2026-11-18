//! The internet module's entry points that GNU R's base-package natives
//! call (`R_ext/R-ftp-http.h`): tools' help server (`startHTTPD`,
//! `stopHTTPD`). robigo-rt has no internet module yet: each raises robigo's
//! "not implemented yet" error.

use crate::entry;
use crate::errors::not_yet;
use std::ffi::{c_char, c_int};

entry! {
    /// `extR_HTTPDCreate(ip, port)`: the help server started.
    // R: internet.c:extR_HTTPDCreate
    [2] fn extR_HTTPDCreate(ip: *const c_char, port: c_int) -> c_int => |cx, _s| {
        let _ = (ip, port);
        Err(not_yet(cx, "the help server (extR_HTTPDCreate)"))
    }

    /// `extR_HTTPDStop()`: the help server stopped.
    // R: internet.c:extR_HTTPDStop
    [0] fn extR_HTTPDStop() -> () => |cx, _s| {
        Err(not_yet(cx, "the help server (extR_HTTPDStop)"))
    }
}
