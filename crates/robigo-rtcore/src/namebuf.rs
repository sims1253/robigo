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

impl From<Vec<u8>> for NameBuf {
    /// A name a slow path already copied (a translated one).
    fn from(v: Vec<u8>) -> NameBuf {
        NameBuf::new(&v)
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
        }
    }
}
