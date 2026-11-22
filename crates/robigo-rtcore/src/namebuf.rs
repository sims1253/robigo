//! [`NameBuf`]: the bytes of a name copied out of a CHARSXP, inline for a
//! short name.
//!
//! GNU R reads a CHARSXP's bytes in place (`translateChar` of an ASCII
//! string is the string itself). robigo-rt cannot hold a borrow of them
//! across the `&mut Cx` calls that follow: a collection may move the
//! string. A dispatch or `$` that copied the name into a `Vec` called the
//! allocator on every evaluation (`malloc`/`free` per S3 dispatch). A
//! `NameBuf` keeps a name of up to [`NameBuf::INLINE`] bytes on the stack;
//! only a longer one is copied to the heap.

use std::ops::Deref;

/// A name's bytes: inline up to [`NameBuf::INLINE`] bytes, else on the heap.
#[derive(Clone)]
pub struct NameBuf {
    len: usize,
    inline: [u8; NameBuf::INLINE],
    heap: Option<Box<[u8]>>,
}

impl NameBuf {
    /// The longest name kept inline.
    pub const INLINE: usize = 64;

    /// A copy of `b`.
    #[inline]
    pub fn new(b: &[u8]) -> NameBuf {
        let mut n = NameBuf { len: b.len(), inline: [0; NameBuf::INLINE], heap: None };
        if b.len() <= NameBuf::INLINE {
            n.inline[..b.len()].copy_from_slice(b);
        } else {
            n.heap = Some(heap_copy(b));
        }
        n
    }
}

/// [`NameBuf::new`]'s copy of a long name (off the common path).
#[cold]
#[inline(never)]
fn heap_copy(b: &[u8]) -> Box<[u8]> {
    b.into()
}

impl Deref for NameBuf {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        match &self.heap {
            Some(h) => h,
            None => &self.inline[..self.len],
        }
    }
}

impl AsRef<[u8]> for NameBuf {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl std::fmt::Debug for NameBuf {
    /// The name's bytes, escaped as a byte string.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NameBuf(b\"{}\")", self.escape_ascii())
    }
}

impl From<Vec<u8>> for NameBuf {
    /// A name a slow path already copied (a translated one): a long one
    /// keeps the Vec's buffer.
    fn from(v: Vec<u8>) -> NameBuf {
        if v.len() <= NameBuf::INLINE {
            NameBuf::new(&v)
        } else {
            NameBuf { len: v.len(), inline: [0; NameBuf::INLINE], heap: Some(v.into_boxed_slice()) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NameBuf;

    #[test]
    fn short_and_long_names_read_back() {
        for n in [0, 1, NameBuf::INLINE, NameBuf::INLINE + 1, 300] {
            let b: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
            assert_eq!(&*NameBuf::new(&b), &b[..]);
            assert_eq!(&*NameBuf::from(b.clone()), &b[..]);
            assert_eq!(NameBuf::new(&b).as_ref(), &b[..]);
        }
        assert_eq!(format!("{:?}", NameBuf::new(b"a\xe9")), "NameBuf(b\"a\\xe9\")");
    }
}
