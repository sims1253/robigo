//! libdeflate's zlib decompressor (vendor/libdeflate, MIT; built by
//! robigo-rt's `build.rs`) behind a safe type: what GNU R 4.6.2's
//! `R_decompress1` calls for the lazy-load databases' records
//! (connections.c, `HAVE_LIBDEFLATE`). Three C functions, each call's
//! contract stated where it is made.
// The C library behind a safe type, docs/architecture.md's "Unsafe code":
// the only way to call C.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::NonNull;

/// `struct libdeflate_decompressor`, opaque.
#[repr(C)]
struct RawDecompressor {
    _opaque: [u8; 0],
}

/// `enum libdeflate_result`'s success (libdeflate.h).
const LIBDEFLATE_SUCCESS: i32 = 0;

unsafe extern "C" {
    fn libdeflate_alloc_decompressor() -> *mut RawDecompressor;
    fn libdeflate_zlib_decompress(
        d: *mut RawDecompressor,
        input: *const c_void,
        in_nbytes: usize,
        out: *mut c_void,
        out_nbytes_avail: usize,
        actual_out_nbytes_ret: *mut usize,
    ) -> i32;
    fn libdeflate_free_decompressor(d: *mut RawDecompressor);
}

/// A libdeflate decompressor: about 32 KiB of tables it reuses from one
/// stream to the next (each call starts afresh: it keeps no stream state).
pub(crate) struct Decompressor(NonNull<RawDecompressor>);

impl Decompressor {
    /// `libdeflate_alloc_decompressor()`: None when it cannot allocate.
    pub(crate) fn new() -> Option<Decompressor> {
        // SAFETY: no arguments; the result is a fresh decompressor or NULL.
        NonNull::new(unsafe { libdeflate_alloc_decompressor() }).map(Decompressor)
    }

    /// `libdeflate_zlib_decompress(d, input, len, out, out.len(),
    /// &actual_out)`: the zlib stream `input` decompressed into `out`,
    /// `Ok(actual_out)` (which may be less than `out.len()`: with a
    /// non-NULL `actual_out` a short stream is a success, never
    /// `LIBDEFLATE_SHORT_OUTPUT`), or `Err(code)` with libdeflate's
    /// `enum libdeflate_result`: 1 `LIBDEFLATE_BAD_DATA`, 3
    /// `LIBDEFLATE_INSUFFICIENT_SPACE`.
    pub(crate) fn zlib_decompress(&mut self, input: &[u8], out: &mut [u8]) -> Result<usize, i32> {
        let mut actual = 0usize;
        // SAFETY: `self.0` is a live decompressor this value owns, used by
        // one call at a time (`&mut self`); `input` is readable for its
        // length and `out` writable for its length, both for the call
        // only (libdeflate keeps neither pointer); `actual` is a writable
        // `size_t`.
        let res = unsafe {
            libdeflate_zlib_decompress(
                self.0.as_ptr(),
                input.as_ptr().cast(),
                input.len(),
                out.as_mut_ptr().cast(),
                out.len(),
                &mut actual,
            )
        };
        if res == LIBDEFLATE_SUCCESS { Ok(actual) } else { Err(res) }
    }
}

impl Drop for Decompressor {
    fn drop(&mut self) {
        // SAFETY: the decompressor `new` allocated, freed once: this value
        // owns it and is dropped once.
        unsafe { libdeflate_free_decompressor(self.0.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::Decompressor;

    /// zlib's stream of `b"hello, hello, hello"` (level 6).
    fn stream() -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
        e.write_all(b"hello, hello, hello").unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn a_stream_decompresses_and_a_short_one_is_a_success() {
        let mut d = Decompressor::new().unwrap();
        let s = stream();
        let mut out = vec![0u8; 19];
        assert_eq!(d.zlib_decompress(&s, &mut out), Ok(19));
        assert_eq!(&out, b"hello, hello, hello");
        // more room than the stream fills: the bytes it has
        let mut big = vec![0u8; 64];
        assert_eq!(d.zlib_decompress(&s, &mut big), Ok(19));
        // too little room: LIBDEFLATE_INSUFFICIENT_SPACE
        let mut small = vec![0u8; 5];
        assert_eq!(d.zlib_decompress(&s, &mut small), Err(3));
        // not a zlib stream: LIBDEFLATE_BAD_DATA; and the decompressor
        // serves the next stream afresh
        assert_eq!(d.zlib_decompress(b"not zlib at all", &mut out), Err(1));
        assert_eq!(d.zlib_decompress(&s, &mut out), Ok(19));
    }
}
