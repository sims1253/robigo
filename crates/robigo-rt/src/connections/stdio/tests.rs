//! The buffer alone, on a fake descriptor: an in-memory file whose system
//! calls are logged and can fail on the Nth write or at close.

use super::*;

const ENOSPC: i32 = 28;

#[derive(Default)]
struct Log {
    /// Each write(2)'s bytes, in order (the failed ones too, empty).
    writes: Vec<Vec<u8>>,
    reads: usize,
    seeks: usize,
}

struct Fake {
    data: Vec<u8>,
    pos: usize,
    blksize: u64,
    chr_tty: bool,
    append: bool,
    /// write(2) calls so far; the one numbered `fail_write` (1-based) and
    /// every later one up to `fail_until` fail with ENOSPC.
    nwrites: usize,
    fail_write: Option<usize>,
    fail_until: usize,
    /// At most this many bytes per write(2) (a short write).
    max_write: usize,
    close_errno: Option<i32>,
    log: std::rc::Rc<std::cell::RefCell<Log>>,
}

impl Fake {
    fn new(blksize: u64) -> Fake {
        Fake {
            data: Vec::new(),
            pos: 0,
            blksize,
            chr_tty: false,
            append: false,
            nwrites: 0,
            fail_write: None,
            fail_until: usize::MAX,
            max_write: usize::MAX,
            close_errno: None,
            log: Default::default(),
        }
    }
    fn with_data(mut self, d: &[u8]) -> Fake {
        self.data = d.to_vec();
        self
    }
}

impl SysFile for Fake {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.log.borrow_mut().reads += 1;
        let n = buf.len().min(self.data.len().saturating_sub(self.pos));
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.nwrites += 1;
        if let Some(k) = self.fail_write
            && self.nwrites >= k
            && self.nwrites <= self.fail_until
        {
            self.log.borrow_mut().writes.push(Vec::new());
            return Err(io::Error::from_raw_os_error(ENOSPC));
        }
        if self.append {
            self.pos = self.data.len();
        }
        let n = buf.len().min(self.max_write);
        if self.data.len() < self.pos + n {
            self.data.resize(self.pos + n, 0);
        }
        self.data[self.pos..self.pos + n].copy_from_slice(&buf[..n]);
        self.pos += n;
        self.log.borrow_mut().writes.push(buf[..n].to_vec());
        Ok(n)
    }
    fn seek(&mut self, off: i64, whence: Whence) -> io::Result<i64> {
        self.log.borrow_mut().seeks += 1;
        let base = match whence {
            Whence::Set => 0,
            Whence::Cur => self.pos as i64,
            Whence::End => self.data.len() as i64,
        };
        let to = base + off;
        if to < 0 {
            return Err(io::Error::from_raw_os_error(EINVAL));
        }
        self.pos = to as usize;
        Ok(to)
    }
    fn stat(&self) -> io::Result<Stat> {
        Ok(Stat { is_reg: !self.chr_tty, is_chr: self.chr_tty, blksize: self.blksize, size: self.data.len() as u64 })
    }
    fn isatty(&self) -> bool {
        self.chr_tty
    }
    fn close(self) -> io::Result<()> {
        match self.close_errno {
            Some(e) => Err(io::Error::from_raw_os_error(e)),
            None => Ok(()),
        }
    }
}

type Shared = std::rc::Rc<std::cell::RefCell<Log>>;

fn open(f: Fake, mode: &str) -> (Stream<Fake>, Shared) {
    let log = f.log.clone();
    (Stream::open(f, mode).unwrap(), log)
}

fn lens(log: &Shared) -> Vec<usize> {
    log.borrow().writes.iter().map(Vec::len).collect()
}

#[test]
fn the_buffer_is_the_block_size_below_bufsiz_else_bufsiz() {
    for (blk, size) in [(4096, 4096), (512, 512), (0, BUFSIZ), (8192, BUFSIZ), (65536, BUFSIZ)] {
        let (mut s, _) = open(Fake::new(blk), "w");
        s.fwrite(b"x");
        assert_eq!(s.buf.len(), size, "st_blksize {blk}");
        assert!(!s.line_buf);
    }
}

#[test]
fn a_terminal_is_line_buffered() {
    let mut f = Fake::new(1024);
    f.chr_tty = true;
    let (mut s, log) = open(f, "w");
    assert_eq!(s.fwrite(b"ab\ncd"), 5);
    // up to the last newline is written at once, the rest stays
    assert_eq!(log.borrow().writes, vec![b"ab\n".to_vec()]);
    assert_eq!(s.fclose(), 0);
    assert_eq!(lens(&log), [3, 2]);
}

#[test]
fn small_writes_stay_in_the_buffer_until_close() {
    let (mut s, log) = open(Fake::new(4096), "w");
    assert_eq!(s.fwrite(b"abc"), 3);
    assert_eq!(s.vfprintf(&[b"de", b"f\n"]), 4);
    assert!(log.borrow().writes.is_empty());
    assert_eq!(s.ftello(), 7);
    assert_eq!(s.fclose(), 0);
    assert_eq!(log.borrow().writes, vec![b"abcdef\n".to_vec()]);
}

#[test]
fn xsputn_writes_whole_blocks_directly_and_buffers_the_rest() {
    let (mut s, log) = open(Fake::new(4096), "wb");
    let data = vec![7u8; 10000];
    assert_eq!(s.fwrite(&data), 10000);
    // a fresh stream: the empty buffer's flush writes nothing, two blocks
    // go directly, 1808 bytes wait
    assert_eq!(lens(&log), [8192]);
    assert_eq!(s.write_ptr - s.write_base, 1808);
    // 100 more fill the buffer partly (1908); 5000 then fill it (2188),
    // flush it, and the remaining 2812 (less than a block) are buffered
    s.fwrite(&[1; 100]);
    s.fwrite(&[2; 5000]);
    assert_eq!(lens(&log), [8192, 4096]);
    assert_eq!(s.write_ptr - s.write_base, 2812);
    assert_eq!(s.fclose(), 0);
    assert_eq!(lens(&log), [8192, 4096, 2812]);
}

#[test]
fn a_failed_flush_drops_the_buffer_and_fwrite_reports_the_short_count() {
    let mut f = Fake::new(4096);
    f.fail_write = Some(1);
    f.fail_until = 1;
    let (mut s, log) = open(f, "wb");
    assert_eq!(s.fwrite(&[1; 4000]), 4000);
    // 96 fit; the flush of the full buffer fails: the 4096 are gone, and
    // fwrite counts what it put before the flush
    assert_eq!(s.fwrite(&[2; 200]), 96);
    assert_eq!(s.errno, ENOSPC);
    assert!(s.err_seen);
    assert_eq!(s.write_ptr, s.write_base);
    // a write after the failure succeeds: the buffer is empty again
    assert_eq!(s.fwrite(b"after"), 5);
    assert_eq!(s.fclose(), 0);
    assert_eq!(log.borrow().writes, vec![Vec::new(), b"after".to_vec()]);
}

#[test]
fn a_failed_direct_write_counts_what_reached_the_file() {
    let mut f = Fake::new(4096);
    f.max_write = 1000;
    f.fail_write = Some(3);
    let (mut s, log) = open(f, "wb");
    // 8192 direct: 1000 + 1000, then ENOSPC
    assert_eq!(s.fwrite(&[1; 9000]), 2000);
    assert_eq!(lens(&log), [1000, 1000, 0]);
    assert_eq!(s.ftello(), 2000);
}

#[test]
fn vfprintf_fills_and_flushes_never_writing_around_the_buffer() {
    let (mut s, log) = open(Fake::new(4096), "w");
    // the first call goes through the stage (no room in the stream yet)
    assert_eq!(s.vfprintf(&[&[b'a'; 200]]), 200);
    assert!(log.borrow().writes.is_empty());
    // exactly filling the buffer does not flush it
    assert_eq!(s.vfprintf(&[&[b'b'; 4096 - 200]]), 3896);
    assert!(log.borrow().writes.is_empty());
    // one more byte does, then 9999 bytes go through it, a buffer at a time
    assert_eq!(s.vfprintf(&[&[b'c'; 10000]]), 10000);
    assert_eq!(lens(&log), [4096, 4096, 4096]);
    assert_eq!(s.write_ptr - s.write_base, 10000 - 8192);
    assert_eq!(s.fclose(), 0);
    assert_eq!(lens(&log), [4096, 4096, 4096, 1808]);
}

#[test]
fn a_failed_vfprintf_is_minus_one_and_the_next_one_succeeds() {
    let mut f = Fake::new(4096);
    f.fail_write = Some(1);
    f.fail_until = 1;
    let (mut s, log) = open(f, "w");
    assert_eq!(s.vfprintf(&[&[b'a'; 4000]]), 4000);
    s.errno = 0;
    assert_eq!(s.vfprintf(&[&[b'b'; 200]]), -1);
    assert_eq!(s.errno, ENOSPC);
    // the rest of the failed call is not written; the next call is
    assert_eq!(s.vfprintf(&[b"ok\n"]), 3);
    assert_eq!(s.fclose(), 0);
    assert_eq!(log.borrow().writes, vec![Vec::new(), b"ok\n".to_vec()]);
}

#[test]
fn vfprintf_to_a_read_only_stream_is_ebadf() {
    let (mut s, _) = open(Fake::new(4096).with_data(b"x"), "r");
    assert_eq!(s.vfprintf(&[b"a"]), -1);
    assert_eq!(s.errno, EBADF);
    assert_eq!(s.fwrite(b"a"), 0);
}

#[test]
fn fclose_reports_the_flush_or_the_close() {
    // the flush fails, the close succeeds: EOF, the write's errno
    let mut f = Fake::new(4096);
    f.fail_write = Some(1);
    let (mut s, _) = open(f, "w");
    s.fwrite(b"abc");
    s.errno = 0;
    assert_eq!(s.fclose(), EOF);
    assert_eq!(s.errno, ENOSPC);
    // the flush succeeds, the close fails: EOF, the close's errno
    let mut f = Fake::new(4096);
    f.close_errno = Some(EIO);
    let (mut s, log) = open(f, "w");
    s.fwrite(b"abc");
    s.errno = 0;
    assert_eq!(s.fclose(), EOF);
    assert_eq!(s.errno, EIO);
    assert_eq!(lens(&log), [3]);
    // both fail: the close's errno is the last
    let mut f = Fake::new(4096);
    f.fail_write = Some(1);
    f.close_errno = Some(EBADF);
    let (mut s, _) = open(f, "w");
    s.fwrite(b"abc");
    assert_eq!(s.fclose(), EOF);
    assert_eq!(s.errno, EBADF);
    // nothing to write after a read
    let (mut s, log) = open(Fake::new(4096).with_data(b"abc"), "r+");
    assert_eq!(s.fgetc(), Some(b'a'));
    assert_eq!(s.fclose(), 0);
    assert!(log.borrow().writes.is_empty());
}

#[test]
fn fflush_writes_and_a_failed_one_is_eof() {
    let mut f = Fake::new(4096);
    f.fail_write = Some(2);
    let (mut s, log) = open(f, "w");
    s.fwrite(b"abc");
    assert_eq!(s.fflush(), 0);
    assert_eq!(lens(&log), [3]);
    assert_eq!(s.fflush(), 0);
    s.fwrite(b"de");
    assert_eq!(s.fflush(), EOF);
    assert_eq!(s.errno, ENOSPC);
    // the dropped bytes are not written again
    assert_eq!(s.fflush(), 0);
}

#[test]
fn reads_fill_the_buffer_and_eof_is_sticky_until_clearerr() {
    let data: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
    let (mut s, log) = open(Fake::new(4096).with_data(&data), "rb");
    let mut b = [0u8; 10];
    assert_eq!(s.fread(&mut b), 10);
    assert_eq!(&b[..], &data[..10]);
    assert_eq!(log.borrow().reads, 1);
    assert_eq!(s.ftello(), 10);
    assert_eq!(s.fgetc(), Some(10));
    // a whole block wanted past the buffer is read directly
    let mut big = vec![0u8; 8192];
    assert_eq!(s.fread(&mut big), 5000 - 11);
    assert!(s.feof());
    assert_eq!(s.fgetc(), None);
    s.clearerr();
    assert!(!s.feof());
}

#[test]
fn a_seek_within_the_read_buffer_reads_nothing_more() {
    let data: Vec<u8> = (0..100u8).collect();
    let (mut s, log) = open(Fake::new(4096).with_data(&data), "r");
    assert_eq!(s.fgetc(), Some(0));
    // the offset is not known yet (fopen's `_IO_pos_BAD`, which a read
    // leaves): the first seek reads the block again
    assert_eq!(s.fseeko(50, Whence::Set), 0);
    assert_eq!(log.borrow().reads, 2);
    let reads = log.borrow().reads;
    assert_eq!(s.fgetc(), Some(50));
    assert_eq!(s.fseeko(-10, Whence::Cur), 0);
    assert_eq!(s.fgetc(), Some(41));
    assert_eq!(log.borrow().reads, reads);
    assert_eq!(s.ftello(), 42);
    // before the start: EINVAL, nothing moves
    assert_eq!(s.fseeko(-100, Whence::Cur), EOF);
    assert_eq!(s.errno, EINVAL);
    assert_eq!(s.ftello(), 42);
}

#[test]
fn switching_from_writing_to_reading_through_fseeko() {
    // file_vfprintf then file_read, as connections.c switches: ftello,
    // fseeko to the other position
    let (mut s, log) = open(Fake::new(4096).with_data(b"0123456789"), "r+");
    assert_eq!(s.fseeko(4, Whence::Set), 0);
    assert_eq!(s.vfprintf(&[b"ab"]), 2);
    assert!(log.borrow().writes.is_empty());
    let wpos = s.ftello();
    assert_eq!(wpos, 6);
    assert_eq!(s.fseeko(0, Whence::Set), 0);
    let mut b = [0u8; 10];
    assert_eq!(s.fread(&mut b), 10);
    assert_eq!(&b, b"0123ab6789");
    assert_eq!(s.fseeko(wpos, Whence::Set), 0);
    assert_eq!(s.fwrite(b"Z"), 1);
    assert_eq!(s.fclose(), 0);
    assert_eq!(log.borrow().writes, vec![b"ab".to_vec(), b"Z".to_vec()]);
}

#[test]
fn append_streams_start_at_the_end_and_tell_past_the_buffered_bytes() {
    let mut f = Fake::new(4096).with_data(b"abc");
    f.append = true;
    let (mut s, _) = open(f, "a");
    assert_eq!(s.ftello(), 3);
    s.fwrite(b"de");
    assert_eq!(s.ftello(), 5);
    assert_eq!(s.fclose(), 0);
    // "a+" is not sent to the end at open
    let mut f = Fake::new(4096).with_data(b"abc");
    f.append = true;
    let (mut s, _) = open(f, "a+");
    assert_eq!(s.ftello(), 0);
}

#[test]
fn fseeko_flushes_pending_output_and_a_failed_flush_is_eof() {
    let mut f = Fake::new(4096);
    f.fail_write = Some(1);
    f.fail_until = 1;
    let (mut s, log) = open(f, "w+");
    s.fwrite(b"lost");
    assert_eq!(s.fseeko(0, Whence::Set), EOF);
    assert_eq!(s.errno, ENOSPC);
    s.fwrite(b"kept");
    assert_eq!(s.fseeko(0, Whence::Set), 0);
    assert_eq!(log.borrow().writes, vec![Vec::new(), b"kept".to_vec()]);
}
