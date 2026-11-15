//! What `robigo-rt` installs behind robigo-rtcore's builtin-author API
//! (R1d): the `EncodeString` an embedded-NUL error shows its string with.

mod common;

use common::*;
use robigo_rtcore::Encoding;

/// mkCharLenCE's "embedded nul in string: '%s'", the string escaped as
/// GNU R's EncodeString(c, 0, 0, Rprt_adj_none) escapes it.
#[test]
fn an_embedded_nul_is_shown_as_gnu_r_shows_it() {
    let mut r = Session::new(robigo_rt::Profile::Eval);
    let cx = r.cx();
    let m = cx.scope(|cx, s| {
        let out = s.out(cx);
        let e = cx.build(out, |cx| cx.mk_string(b"a\0b\n", Encoding::Native)).unwrap_err();
        message(cx, e)
    });
    assert_eq!(m, "embedded nul in string: 'a\\0b\\n'");
    // (a printable character as it is, in the UTF-8 locale the oracle
    // records in: `rawToChar(as.raw(c(0xc3, 0xa9, 0, 9)))` on GNU R 4.6.2)
    let m = cx.scope(|cx, s| {
        let out = s.out(cx);
        let e = cx.build(out, |cx| cx.mk_string(b"\xc3\xa9\0\t", Encoding::Native)).unwrap_err();
        message(cx, e)
    });
    assert_eq!(m, "embedded nul in string: 'é\\0\\t'");
    // a "bytes" string shows up to its first NUL (EncodeString's strlen)
    let m = cx.scope(|cx, s| {
        let out = s.out(cx);
        let e = cx.build(out, |cx| cx.mk_string(b"a\0b", Encoding::Bytes)).unwrap_err();
        message(cx, e)
    });
    assert_eq!(m, "embedded nul in string: 'a'");
}
