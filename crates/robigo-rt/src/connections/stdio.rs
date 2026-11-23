//! The C library's buffered stream (`FILE`) under a file connection, as GNU
//! R's `file()` has it: connections.c's `file_*` methods call `fopen`,
//! `fwrite`, `vfprintf`, `fread`, `fgetc`, `ftello`, `fseeko`, `fflush` and
//! `fclose`, and what reaches the file, when, and which call reports a failed
//! write follow from glibc's buffer. This is a port of that buffer, read in
//! glibc 2.41 (`libio/fileops.c`, `libio/genops.c`, `libio/filedoalloc.c`,
//! `libio/iofwrite.c`, `libio/iofread.c`, `libio/iofflush.c`,
//! `libio/iofclose.c`, `libio/ioftell.c`, `libio/ioseekoff.c`,
//! `stdio-common/printf_buffer_to_file.c` and the `Xprintf_buffer_*` it
//! uses), for the narrow, byte-oriented stream connections.c opens: no
//! wide orientation, no push-back (`ungetc`) or backup area, no markers, no
//! `mmap` mode, no locking (a stream is the isolate's own), no `setvbuf`
//! (GNU R calls none, so the buffer is `_IO_file_doallocate`'s).
//!
//! The descriptor is a [`SysFile`], so the unit tests can make the system
//! fail on the Nth call. `errno` is the stream's own: [`Stream::errno`] is
//! what the system call that failed last set (or what glibc set itself), and
//! its callers clear it first as GNU R clears the C library's.
//!
//! glibc's buffer pointers are indices into [`Stream::buf`]; a stream with
//! no buffer yet has them all 0, as glibc's are all NULL.

use std::io::{self, SeekFrom};

/// `BUFSIZ`.
const BUFSIZ: usize = 8192;
/// `PRINTF_BUFFER_SIZE_TO_FILE_STAGE` (include/printf_buffer.h).
const STAGE: usize = 128;
/// `EOF`.
pub(crate) const EOF: i32 = -1;
/// `_IO_pos_BAD`.
const POS_BAD: i64 = -1;
/// `(size_t) EOF`: what `_IO_new_file_xsputn` returns when the flush of a
/// line-buffered stream failed after every byte was taken.
const EOF_SIZE: usize = usize::MAX;

/// `SEEK_SET`, `SEEK_CUR`, `SEEK_END` (`_IO_seek_set`, ...).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Whence {
    Set,
    Cur,
    End,
}

/// What `fstat` tells the stream.
pub(crate) struct Stat {
    pub is_reg: bool,
    pub is_chr: bool,
    pub blksize: u64,
    pub size: u64,
}

/// The descriptor's system calls (glibc's `_IO_SYSREAD`, `_IO_SYSWRITE`,
/// `_IO_SYSSEEK`, `_IO_SYSSTAT`, `_IO_SYSCLOSE`, and `isatty`): one call
/// each, an error is the call's `errno`.
pub(crate) trait SysFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    fn write(&mut self, buf: &[u8]) -> io::Result<usize>;
    /// `lseek(fd, off, whence)`: the new offset.
    fn seek(&mut self, off: i64, whence: Whence) -> io::Result<i64>;
    fn stat(&self) -> io::Result<Stat>;
    fn isatty(&self) -> bool;
    fn close(self) -> io::Result<()>;
}

impl SysFile for std::fs::File {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        io::Read::read(self, buf)
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::Write::write(self, buf)
    }
    fn seek(&mut self, off: i64, whence: Whence) -> io::Result<i64> {
        let to = match whence {
            Whence::Set => match u64::try_from(off) {
                Ok(o) => SeekFrom::Start(o),
                // lseek's EINVAL for a negative offset
                Err(_) => return Err(io::Error::from_raw_os_error(EINVAL)),
            },
            Whence::Cur => SeekFrom::Current(off),
            Whence::End => SeekFrom::End(off),
        };
        io::Seek::seek(self, to).map(|p| p as i64)
    }
    fn stat(&self) -> io::Result<Stat> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let m = self.metadata()?;
        Ok(Stat { is_reg: m.is_file(), is_chr: m.file_type().is_char_device(), blksize: m.blksize(), size: m.size() })
    }
    fn isatty(&self) -> bool {
        io::IsTerminal::is_terminal(self)
    }
    fn close(self) -> io::Result<()> {
        robigo_rtcore::clib::close_file(self)
    }
}

/// The errno values glibc sets itself.
const EBADF: i32 = 9;
const EIO: i32 = 5;
const EINVAL: i32 = 22;
const ESPIPE: i32 = 29;
const EOVERFLOW: i32 = 75;

/// The streams opened so far in the process: [`Stream::opened`]'s order
/// (glibc's `_IO_list_all`, which `_IO_link_in` prepends each new stream to).
static OPENED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A buffered stream (glibc's `FILE` for a file opened by `fopen`).
pub(crate) struct Stream<F: SysFile> {
    /// When it was opened, in the process's order of opening.
    opened: u64,
    /// `_fileno`; None once closed.
    fd: Option<F>,
    /// `_IO_buf_base` .. `_IO_buf_end`: empty until allocated.
    buf: Vec<u8>,
    read_base: usize,
    read_ptr: usize,
    read_end: usize,
    write_base: usize,
    write_ptr: usize,
    write_end: usize,
    /// `_offset`: the descriptor's offset when known, else `POS_BAD`.
    offset: i64,
    // `_flags`
    no_reads: bool,
    no_writes: bool,
    is_appending: bool,
    currently_putting: bool,
    eof_seen: bool,
    err_seen: bool,
    line_buf: bool,
    /// `errno` as the stream's calls leave it.
    pub errno: i32,
    /// `vfprintf`'s stage (`struct __printf_buffer_to_file`'s, on glibc's
    /// stack): kept here so a call does not initialize it.
    stage: [u8; STAGE],
}

/// Where `new_do_write` takes its bytes from: the stream's buffer, or the
/// caller's.
enum Data<'a> {
    Buf(usize, usize),
    Ext(&'a [u8]),
}

impl<F: SysFile> Stream<F> {
    /// `fopen(name, mode)` once the descriptor is open: the mode's flags,
    /// and a write-only append stream sent to the end of the file. Err is
    /// the seek's failure (glibc closes the descriptor and `fopen` fails).
    // glibc: libio/fileops.c:_IO_new_file_fopen, _IO_file_open
    pub(crate) fn open(fd: F, mode: &str) -> io::Result<Stream<F>> {
        let m = mode.as_bytes();
        let (mut no_reads, mut no_writes, is_appending) = match m.first() {
            Some(b'r') => (false, true, false),
            Some(b'w') => (true, false, false),
            Some(b'a') => (true, false, true),
            _ => return Err(io::Error::from_raw_os_error(EINVAL)),
        };
        for &ch in m.iter().skip(1).take(6) {
            match ch {
                b',' => break,
                // read_write &= _IO_IS_APPENDING
                b'+' => {
                    no_reads = false;
                    no_writes = false;
                }
                _ => {}
            }
        }
        let mut s = Stream {
            opened: OPENED.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            fd: Some(fd),
            buf: Vec::new(),
            read_base: 0,
            read_ptr: 0,
            read_end: 0,
            write_base: 0,
            write_ptr: 0,
            write_end: 0,
            offset: POS_BAD,
            no_reads,
            no_writes,
            is_appending,
            currently_putting: false,
            eof_seen: false,
            err_seen: false,
            line_buf: false,
            errno: 0,
            stage: [0; STAGE],
        };
        // For append mode, send the file offset to the end of the file.
        // Don't update the offset cache though.
        if is_appending && no_reads {
            let fd = s.fd.as_mut().expect("open");
            if let Err(e) = fd.seek(0, Whence::End)
                && e.raw_os_error() != Some(ESPIPE)
            {
                if let Some(fd) = s.fd.take() {
                    let _ = fd.close();
                }
                return Err(e);
            }
        }
        Ok(s)
    }

    /// The descriptor, for the calls connections.c makes on `fileno(fp)`.
    pub(crate) fn fd(&mut self) -> &mut F {
        self.fd.as_mut().expect("an open stream")
    }

    fn allocated(&self) -> bool {
        !self.buf.is_empty()
    }

    fn buf_end(&self) -> usize {
        self.buf.len()
    }

    fn set_errno(&mut self, e: &io::Error) {
        // a fake's error without a code is an I/O error
        self.errno = e.raw_os_error().unwrap_or(EIO);
    }

    // glibc: libio/libioP.h:_IO_setg
    fn setg(&mut self, eb: usize, g: usize, eg: usize) {
        self.read_base = eb;
        self.read_ptr = g;
        self.read_end = eg;
    }

    // glibc: libio/libioP.h:_IO_setp
    fn setp(&mut self, p: usize, ep: usize) {
        self.write_base = p;
        self.write_ptr = p;
        self.write_end = ep;
    }

    /// `_IO_SYSSEEK`: the new offset, or `POS_BAD` with `errno`.
    // glibc: libio/fileops.c:_IO_file_seek
    fn sys_seek(&mut self, off: i64, whence: Whence) -> i64 {
        match self.fd().seek(off, whence) {
            Ok(p) => p,
            Err(e) => {
                self.set_errno(&e);
                POS_BAD
            }
        }
    }

    /// `_IO_SYSREAD`: the count, 0 at end of file, -1 with `errno`.
    // glibc: libio/fileops.c:_IO_file_read
    fn sys_read(&mut self, from: usize, to: usize, into: Option<&mut [u8]>) -> isize {
        let fd = self.fd.as_mut().expect("an open stream");
        let r = match into {
            Some(out) => fd.read(out),
            None => fd.read(&mut self.buf[from..to]),
        };
        match r {
            Ok(n) => n as isize,
            Err(e) => {
                self.set_errno(&e);
                -1
            }
        }
    }

    /// Allocate a file buffer: `BUFSIZ`, or the file's block size when it
    /// is smaller; a terminal's stream is line buffered.
    // glibc: libio/filedoalloc.c:_IO_file_doallocate
    fn doallocate(&mut self) {
        let mut size = BUFSIZ;
        if let Ok(st) = self.fd().stat() {
            // Possibly a tty (`local_isatty` keeps errno).
            if st.is_chr && self.fd().isatty() {
                self.line_buf = true;
            }
            if st.blksize > 0 && st.blksize < BUFSIZ as u64 {
                size = st.blksize as usize;
            }
        }
        // _IO_setb
        self.buf = vec![0; size];
    }

    // glibc: libio/genops.c:_IO_doallocbuf
    fn doallocbuf(&mut self) {
        if self.allocated() {
            return;
        }
        self.doallocate();
    }

    /// Write `data` to the descriptor, retrying short writes; stop at the
    /// first failure (`_IO_ERR_SEEN`). The bytes written.
    // glibc: libio/fileops.c:_IO_new_file_write
    fn file_write(&mut self, data: Data<'_>) -> usize {
        let fd = self.fd.as_mut().expect("an open stream");
        let bytes: &[u8] = match data {
            Data::Buf(a, b) => &self.buf[a..b],
            Data::Ext(s) => s,
        };
        let n = bytes.len();
        let mut to_do = n;
        let mut failed = None;
        while to_do > 0 {
            // (only a failure stops: a write of nothing is tried again)
            match fd.write(&bytes[n - to_do..]) {
                Ok(count) => to_do -= count,
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = failed {
            self.err_seen = true;
            self.set_errno(&e);
        }
        let n = n - to_do;
        if self.offset >= 0 {
            self.offset += n as i64;
        }
        n
    }

    /// Write `data`, then mark the buffer empty (whether or not the write
    /// succeeded: what a failed write held is dropped).
    // glibc: libio/fileops.c:new_do_write
    fn new_do_write(&mut self, data: Data<'_>) -> usize {
        if self.is_appending {
            // the offset (before and after) is unpredictable
            self.offset = POS_BAD;
        } else if self.read_end != self.write_base {
            let new_pos = self.sys_seek(self.write_base as i64 - self.read_end as i64, Whence::Cur);
            if new_pos == POS_BAD {
                return 0;
            }
            self.offset = new_pos;
        }
        let count = self.file_write(data);
        self.setg(0, 0, 0);
        self.write_base = 0;
        self.write_ptr = 0;
        self.write_end = if self.line_buf { 0 } else { self.buf_end() };
        count
    }

    // glibc: libio/fileops.c:_IO_new_do_write
    fn do_write(&mut self, from: usize, to: usize) -> i32 {
        let to_do = to - from;
        if to_do == 0 || self.new_do_write(Data::Buf(from, to)) == to_do { 0 } else { EOF }
    }

    // glibc: libio/libioP.h:_IO_do_flush
    fn do_flush(&mut self) -> i32 {
        self.do_write(self.write_base, self.write_ptr)
    }

    /// Put `ch` (None: only flush), switching to writing first.
    // glibc: libio/fileops.c:_IO_new_file_overflow
    fn overflow(&mut self, ch: Option<u8>) -> i32 {
        if self.no_writes {
            self.err_seen = true;
            self.errno = EBADF;
            return EOF;
        }
        // If currently reading or no buffer allocated.
        if !self.currently_putting || !self.allocated() {
            if !self.allocated() {
                self.doallocbuf();
                self.setg(0, 0, 0);
            }
            // If the read pointer is at the buffer end, slide the buffer
            // forwards one block; otherwise leave the read end, which
            // corresponds to the external position.
            if self.read_ptr == self.buf_end() {
                self.read_end = 0;
                self.read_ptr = 0;
            }
            self.write_ptr = self.read_ptr;
            self.write_base = self.write_ptr;
            self.write_end = self.buf_end();
            self.read_base = self.read_end;
            self.read_ptr = self.read_end;
            self.currently_putting = true;
            if self.line_buf {
                self.write_end = self.write_ptr;
            }
        }
        let Some(ch) = ch else {
            return self.do_write(self.write_base, self.write_ptr);
        };
        // Buffer is really full
        if self.write_ptr == self.buf_end() && self.do_flush() == EOF {
            return EOF;
        }
        self.buf[self.write_ptr] = ch;
        self.write_ptr += 1;
        if self.line_buf && ch == b'\n' && self.do_write(self.write_base, self.write_ptr) == EOF {
            return EOF;
        }
        ch as i32
    }

    // glibc: libio/genops.c:_IO_switch_to_get_mode
    fn switch_to_get_mode(&mut self) -> i32 {
        if self.write_ptr > self.write_base && self.overflow(None) == EOF {
            return EOF;
        }
        self.read_base = 0;
        if self.write_ptr > self.read_end {
            self.read_end = self.write_ptr;
        }
        self.read_ptr = self.write_ptr;
        self.write_base = self.read_ptr;
        self.write_ptr = self.read_ptr;
        self.write_end = self.read_ptr;
        self.currently_putting = false;
        0
    }

    /// Fill the buffer from the descriptor: its first byte, or None at end
    /// of file or on an error.
    // glibc: libio/fileops.c:_IO_new_file_underflow
    fn file_underflow(&mut self) -> Option<u8> {
        // C99 requires EOF to be "sticky".
        if self.eof_seen {
            return None;
        }
        if self.no_reads {
            self.err_seen = true;
            self.errno = EBADF;
            return None;
        }
        if self.read_ptr < self.read_end {
            return Some(self.buf[self.read_ptr]);
        }
        if !self.allocated() {
            self.doallocbuf();
        }
        // (a line-buffered stream's read flushes the C library's stdout
        // first: robigo's console output is not written through it)
        self.switch_to_get_mode();
        self.read_base = 0;
        self.read_ptr = 0;
        self.read_end = 0;
        self.write_base = 0;
        self.write_ptr = 0;
        self.write_end = 0;
        let mut count = self.sys_read(0, self.buf_end(), None);
        if count <= 0 {
            if count == 0 {
                self.eof_seen = true;
            } else {
                self.err_seen = true;
                count = 0;
            }
        }
        self.read_end += count as usize;
        if count == 0 {
            // the offset cache is no longer valid
            self.offset = POS_BAD;
            return None;
        }
        if self.offset != POS_BAD {
            self.offset += count as i64;
        }
        Some(self.buf[self.read_ptr])
    }

    // glibc: libio/genops.c:__underflow
    fn underflow(&mut self) -> Option<u8> {
        if self.currently_putting && self.switch_to_get_mode() == EOF {
            return None;
        }
        if self.read_ptr < self.read_end {
            return Some(self.buf[self.read_ptr]);
        }
        self.file_underflow()
    }

    /// `fgetc(fp)`: the next byte, None at end of file or on an error.
    // glibc: libio/getc.c:_IO_getc, libio/genops.c:__uflow, _IO_default_uflow
    pub(crate) fn fgetc(&mut self) -> Option<u8> {
        if self.read_ptr < self.read_end {
            let c = self.buf[self.read_ptr];
            self.read_ptr += 1;
            return Some(c);
        }
        if self.currently_putting && self.switch_to_get_mode() == EOF {
            return None;
        }
        if self.read_ptr < self.read_end {
            let c = self.buf[self.read_ptr];
            self.read_ptr += 1;
            return Some(c);
        }
        let c = self.file_underflow()?;
        self.read_ptr += 1;
        Some(c)
    }

    /// Copy what fits into the buffer, overflowing a byte at a time.
    // glibc: libio/genops.c:_IO_default_xsputn
    fn default_xsputn(&mut self, data: &[u8]) -> usize {
        let n = data.len();
        let mut more = n;
        let mut s = 0;
        if more == 0 {
            return 0;
        }
        loop {
            if self.write_ptr < self.write_end {
                let count = (self.write_end - self.write_ptr).min(more);
                self.buf[self.write_ptr..self.write_ptr + count].copy_from_slice(&data[s..s + count]);
                self.write_ptr += count;
                s += count;
                more -= count;
            }
            if more == 0 {
                break;
            }
            let ch = data[s];
            s += 1;
            if self.overflow(Some(ch)) == EOF {
                break;
            }
            more -= 1;
        }
        n - more
    }

    /// Fill the buffer; when it is full, flush it, write whole blocks of
    /// the rest directly and buffer the remainder. The bytes taken (a
    /// failed flush or write stops short), or `EOF_SIZE`.
    // glibc: libio/fileops.c:_IO_new_file_xsputn
    fn xsputn(&mut self, data: &[u8]) -> usize {
        let n = data.len();
        let mut s = 0;
        let mut to_do = n;
        let mut must_flush = false;
        let mut count = 0;
        if n == 0 {
            return 0;
        }
        // First figure out how much space is available in the buffer.
        if self.line_buf && self.currently_putting {
            count = self.buf_end() - self.write_ptr;
            if count >= n
                && let Some(p) = data.iter().rposition(|&b| b == b'\n')
            {
                count = p + 1;
                must_flush = true;
            }
        } else if self.write_end > self.write_ptr {
            count = self.write_end - self.write_ptr;
        }
        // Then fill the buffer.
        if count > 0 {
            count = count.min(to_do);
            self.buf[self.write_ptr..self.write_ptr + count].copy_from_slice(&data[..count]);
            self.write_ptr += count;
            s += count;
            to_do -= count;
        }
        if to_do + usize::from(must_flush) > 0 {
            // Next flush the (full) buffer.
            if self.overflow(None) == EOF {
                // If nothing else has to be written we must not signal the
                // caller that everything has been written.
                return if to_do == 0 { EOF_SIZE } else { n - to_do };
            }
            // Try to maintain alignment: write a whole number of blocks.
            let block_size = self.buf_end();
            let do_write = to_do - if block_size >= 128 { to_do % block_size } else { 0 };
            if do_write > 0 {
                let count = self.new_do_write(Data::Ext(&data[s..s + do_write]));
                to_do -= count;
                if count < do_write {
                    return n - to_do;
                }
            }
            // Now write out the remainder.
            if to_do > 0 {
                to_do -= self.default_xsputn(&data[s + do_write..s + do_write + to_do]);
            }
        }
        n - to_do
    }

    /// `fwrite(data, 1, n, fp)`: the bytes written, as `fwrite` counts them
    /// (all of them when they are in the buffer).
    // glibc: libio/iofwrite.c:_IO_fwrite
    pub(crate) fn fwrite(&mut self, data: &[u8]) -> usize {
        let request = data.len();
        if request == 0 {
            return 0;
        }
        let written = self.xsputn(data);
        if written == request || written == EOF_SIZE { request } else { written }
    }

    /// Read into `out` from the buffer, refilling it, or directly for whole
    /// blocks. The bytes read.
    // glibc: libio/fileops.c:_IO_file_xsgetn
    fn xsgetn(&mut self, out: &mut [u8]) -> usize {
        let n = out.len();
        let mut want = n;
        let mut s = 0;
        if !self.allocated() {
            self.doallocbuf();
        }
        while want > 0 {
            let have = self.read_end - self.read_ptr;
            if want <= have {
                out[s..s + want].copy_from_slice(&self.buf[self.read_ptr..self.read_ptr + want]);
                self.read_ptr += want;
                want = 0;
            } else {
                if have > 0 {
                    out[s..s + have].copy_from_slice(&self.buf[self.read_ptr..self.read_end]);
                    s += have;
                    want -= have;
                    self.read_ptr += have;
                }
                // If we now want less than a buffer, underflow and repeat
                // the copy. Otherwise, read directly to the user buffer.
                if self.allocated() && want < self.buf_end() {
                    if self.underflow().is_none() {
                        break;
                    }
                    continue;
                }
                self.setg(0, 0, 0);
                self.setp(0, 0);
                // Try to maintain alignment: read a whole number of blocks.
                let mut count = want;
                let block_size = self.buf_end();
                if block_size >= 128 {
                    count -= want % block_size;
                }
                let count = self.sys_read(0, 0, Some(&mut out[s..s + count]));
                if count <= 0 {
                    if count == 0 {
                        self.eof_seen = true;
                    } else {
                        self.err_seen = true;
                    }
                    break;
                }
                s += count as usize;
                want -= count as usize;
                if self.offset != POS_BAD {
                    self.offset += count as i64;
                }
            }
        }
        n - want
    }

    /// `fread(out, 1, n, fp)`: the bytes read.
    // glibc: libio/iofread.c:_IO_fread
    pub(crate) fn fread(&mut self, out: &mut [u8]) -> usize {
        if out.is_empty() {
            return 0;
        }
        self.xsgetn(out)
    }

    /// `vfprintf(fp, ...)` of the bytes `parts` (what the format made, in
    /// order): written into the stream's buffer as they come, the buffer
    /// flushed when it is full (through `__overflow`, one more byte), never
    /// written around it. The count, or -1 when a flush failed (what was in
    /// the buffer is dropped, the rest of the output not written).
    // glibc: stdio-common/vfprintf-internal.c:__vfprintf_internal,
    //        stdio-common/printf_buffer_to_file.c (the stage is used while
    //        the stream has no room: not writing, or no buffer yet)
    pub(crate) fn vfprintf(&mut self, parts: &[&[u8]]) -> i32 {
        // ARGCHECK
        if self.no_writes {
            self.err_seen = true;
            self.errno = EBADF;
            return -1;
        }
        let total: usize = parts.iter().map(|p| p.len()).sum();
        if total <= self.room()
            && let Ok(n) = i32::try_from(total)
        {
            for p in parts {
                self.put(p);
            }
            return n;
        }
        let mut pb = PrintfBuf::init(self);
        for p in parts {
            pb.write(self, p);
        }
        pb.done(self)
    }

    /// The room the buffer has left for writing: output up to this long
    /// that `vfprintf` writes is only copied into the buffer ([`Self::put`]).
    fn room(&self) -> usize {
        self.write_end.saturating_sub(self.write_ptr)
    }

    /// What `vfprintf` does with output that fits in [`Self::room`]: the
    /// bytes copied into the buffer, no flush (a fill to the end leaves it
    /// full, for the next write to flush). `vfprintf`'s checks are the
    /// caller's: the stream is writing (it has room).
    fn put(&mut self, b: &[u8]) {
        debug_assert!(b.len() <= self.room());
        self.buf[self.write_ptr..self.write_ptr + b.len()].copy_from_slice(b);
        self.write_ptr += b.len();
    }

    /// `fflush(fp)`: the buffered output written, a read's buffered input
    /// given back to the descriptor. 0 or `EOF`.
    // glibc: libio/iofflush.c:_IO_fflush, libio/fileops.c:_IO_new_file_sync
    pub(crate) fn fflush(&mut self) -> i32 {
        let mut retval = 0;
        if self.write_ptr > self.write_base && self.do_flush() != 0 {
            return EOF;
        }
        let delta = self.read_ptr as i64 - self.read_end as i64;
        if delta != 0 {
            let new_pos = self.sys_seek(delta, Whence::Cur);
            if new_pos != POS_BAD {
                self.read_end = self.read_ptr;
            } else if self.errno == ESPIPE {
                // Ignore error from unseekable devices.
            } else {
                retval = EOF;
            }
        }
        if retval != EOF {
            self.offset = POS_BAD;
        }
        retval
    }

    /// `ftello(fp)`: the descriptor's offset with the buffer's bytes
    /// counted, or -1 with `errno`.
    // glibc: libio/ioftell.c:_IO_ftell, libio/fileops.c:do_ftell
    pub(crate) fn ftello(&mut self) -> i64 {
        let pos = self.do_ftell();
        if pos == POS_BAD {
            if self.errno == 0 {
                self.errno = EIO;
            }
            return -1;
        }
        pos
    }

    // glibc: libio/fileops.c:do_ftell
    fn do_ftell(&mut self) -> i64 {
        let mut offset: i64 = 0;
        // No point looking at unflushed data if we haven't allocated
        // buffers yet.
        if self.allocated() {
            let unflushed_writes = self.write_ptr > self.write_base;
            let append_mode = self.is_appending;
            // When we have unflushed writes in append mode, seek to the end
            // of the file and record that offset.
            if unflushed_writes && append_mode {
                let result = self.sys_seek(0, Whence::End);
                if result == POS_BAD {
                    return EOF as i64;
                }
                self.offset = result;
            }
            // Adjust for unflushed data.
            if !unflushed_writes {
                offset -= self.read_end as i64 - self.read_ptr as i64;
            } else if append_mode {
                offset += self.write_ptr as i64 - self.write_base as i64;
            } else {
                offset += self.write_ptr as i64 - self.read_end as i64;
            }
        }
        let result = if self.offset != POS_BAD { self.offset } else { self.sys_seek(0, Whence::Cur) };
        if result == EOF as i64 {
            return result;
        }
        let result = result + offset;
        if result < 0 {
            self.errno = EINVAL;
            return EOF as i64;
        }
        result
    }

    /// `fseeko(fp, offset, whence)`: 0, or -1 with `errno`.
    // glibc: libio/fseeko.c:__fseeko, libio/iolibio.h:_IO_fseek,
    //        libio/ioseekoff.c:_IO_seekoff_unlocked
    pub(crate) fn fseeko(&mut self, offset: i64, whence: Whence) -> i32 {
        if self.seekoff(offset, whence) == POS_BAD { EOF } else { 0 }
    }

    /// Move the stream (reading and writing) to `offset` from `dir`: the
    /// new position, or `POS_BAD`. Buffered output is written first; a
    /// destination within the read buffer moves only the pointers.
    // glibc: libio/fileops.c:_IO_new_file_seekoff
    fn seekoff(&mut self, mut offset: i64, mut dir: Whence) -> i64 {
        // POSIX.1 8.2.3.7 says that after a call the fflush() the file
        // offset of the underlying file must be exact.
        let must_be_exact = self.read_base == self.read_end && self.write_base == self.write_ptr;
        let was_writing = self.write_ptr > self.write_base || self.currently_putting;
        // Flush unwritten characters.
        if was_writing && self.switch_to_get_mode() != 0 {
            return POS_BAD;
        }
        if !self.allocated() {
            self.doallocbuf();
            self.setp(0, 0);
            self.setg(0, 0, 0);
        }
        'dumb: {
            match dir {
                Whence::Cur => {
                    // Adjust for read-ahead (bytes is buffer).
                    offset -= self.read_end as i64 - self.read_ptr as i64;
                    if self.offset == POS_BAD {
                        break 'dumb;
                    }
                    // Make offset absolute, assuming current pointer is
                    // file_ptr().
                    offset += self.offset;
                    if offset < 0 {
                        self.errno = EINVAL;
                        return POS_BAD;
                    }
                    dir = Whence::Set;
                }
                Whence::Set => {}
                Whence::End => match self.fd().stat() {
                    Ok(st) if st.is_reg => {
                        offset += st.size as i64;
                        dir = Whence::Set;
                    }
                    _ => break 'dumb,
                },
            }
            // At this point, dir==_IO_seek_set.
            // If destination is within current buffer, optimize:
            if self.offset != POS_BAD && self.allocated() {
                let start_offset = self.offset - self.read_end as i64;
                if offset >= start_offset && offset < self.offset {
                    self.setg(0, (offset - start_offset) as usize, self.read_end);
                    self.setp(0, 0);
                    self.eof_seen = false;
                    // resync: the kernel's offset may have been changed
                    // behind our back
                    if self.offset >= 0 {
                        self.sys_seek(self.offset, Whence::Set);
                    }
                    return offset;
                }
            }
            if self.no_reads {
                break 'dumb;
            }
            // Try to seek to a block boundary, to improve kernel page
            // management.
            let bufsize = self.buf_end() as i64;
            let mut new_offset = offset & !(bufsize - 1);
            let mut delta = offset - new_offset;
            if delta > bufsize {
                new_offset = offset;
                delta = 0;
            }
            let result = self.sys_seek(new_offset, Whence::Set);
            if result < 0 {
                return POS_BAD;
            }
            let count = if delta == 0 {
                0
            } else {
                let want = if must_be_exact { delta as usize } else { self.buf_end() };
                let count = self.sys_read(0, want, None);
                if (count as i64) < delta {
                    // We weren't allowed to read, but try to seek the
                    // remainder.
                    offset = if count == -1 { delta } else { delta - count as i64 };
                    dir = Whence::Cur;
                    break 'dumb;
                }
                count as usize
            };
            self.setg(0, delta as usize, count);
            self.setp(0, 0);
            self.offset = result + count as i64;
            self.eof_seen = false;
            return offset;
        }
        // dumb:
        let result = self.sys_seek(offset, dir);
        if result != POS_BAD {
            self.eof_seen = false;
            self.offset = result;
            self.setg(0, 0, 0);
            self.setp(0, 0);
        }
        result
    }

    /// `feof(fp)`.
    pub(crate) fn feof(&self) -> bool {
        self.eof_seen
    }

    /// `clearerr(fp)`.
    pub(crate) fn clearerr(&mut self) {
        self.eof_seen = false;
        self.err_seen = false;
    }

    /// Its place in the order of opening: `exit` flushes the newest first.
    // glibc: libio/genops.c:_IO_link_in, _IO_flush_all
    pub(crate) fn opened(&self) -> u64 {
        self.opened
    }

    /// What `exit` does for each open stream: the buffered output written,
    /// a failure ignored (the descriptor stays open, for the system to
    /// close).
    // glibc: libio/genops.c:_IO_cleanup, _IO_flush_all
    pub(crate) fn flush_at_exit(&mut self) {
        if self.fd.is_some() && self.write_ptr > self.write_base {
            self.overflow(None);
        }
    }

    /// `fclose(fp)`: the buffered output written, the descriptor closed.
    /// 0, or `EOF` when either failed (`errno` the last failure's).
    // glibc: libio/iofclose.c:_IO_new_fclose,
    //        libio/fileops.c:_IO_new_file_close_it
    pub(crate) fn fclose(&mut self) -> i32 {
        if self.fd.is_none() {
            return EOF;
        }
        let write_status = if !self.no_writes && self.currently_putting { self.do_flush() } else { 0 };
        let close_status = match self.fd.take().expect("open").close() {
            Ok(()) => 0,
            Err(e) => {
                self.set_errno(&e);
                -1
            }
        };
        // Free buffer.
        self.buf = Vec::new();
        self.setg(0, 0, 0);
        self.setp(0, 0);
        self.offset = POS_BAD;
        if close_status != 0 { close_status } else { write_status }
    }
}

/// A stream dropped open (its isolate's connections dropped without
/// closing them) writes what it holds, as the process's exit would.
impl<F: SysFile> Drop for Stream<F> {
    fn drop(&mut self) {
        self.flush_at_exit();
    }
}

/// Which buffer `vfprintf`'s output goes to (`struct
/// __printf_buffer_to_file`'s `write_end`): the stream's, the stage, or the
/// stage's first byte (once the stream's is full, the byte `__overflow`
/// puts after flushing it).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    File,
    Stage,
    One,
}

/// `struct __printf_buffer_to_file`: the write pointers into the target,
/// the bytes consumed, and failure.
struct PrintfBuf {
    target: Target,
    write_base: usize,
    write_ptr: usize,
    write_end: usize,
    written: usize,
    failed: bool,
}

impl PrintfBuf {
    // glibc: stdio-common/printf_buffer_to_file.c:__printf_buffer_to_file_init
    fn init<F: SysFile>(fp: &Stream<F>) -> PrintfBuf {
        let mut b = PrintfBuf {
            target: Target::Stage,
            write_base: 0,
            write_ptr: 0,
            write_end: STAGE,
            written: 0,
            failed: false,
        };
        b.switch(fp);
        b
    }

    /// Switch to the file buffer if possible, else the stage.
    // glibc: stdio-common/printf_buffer_to_file.c:__printf_buffer_to_file_switch
    fn switch<F: SysFile>(&mut self, fp: &Stream<F>) {
        if fp.write_ptr < fp.write_end {
            self.target = Target::File;
            self.write_ptr = fp.write_ptr;
            self.write_end = fp.write_end;
        } else {
            self.target = Target::Stage;
            self.write_ptr = 0;
            self.write_end = STAGE;
        }
        self.write_base = self.write_ptr;
    }

    // glibc: stdio-common/printf_buffer_to_file.c:__printf_buffer_flush_to_file
    fn flush_to_file<F: SysFile>(&mut self, fp: &mut Stream<F>) {
        // The bytes in the buffer are always consumed.
        self.written += self.write_ptr - self.write_base;
        match self.target {
            Target::Stage => {
                // The stage buffer is always consumed fully, even if just
                // partially written.
                let count = self.write_ptr;
                let stage = fp.stage;
                if fp.xsputn(&stage[..count]) != count {
                    self.failed = true;
                    return;
                }
                // fp may have a buffer now.
                self.switch(fp);
            }
            Target::One => {
                if self.write_ptr == self.write_end {
                    if fp.overflow(Some(fp.stage[0])) == EOF {
                        self.failed = true;
                        return;
                    }
                    self.switch(fp);
                }
            }
            Target::File => {
                // Mark the bytes as written.
                fp.write_ptr = self.write_ptr;
                if self.write_ptr == self.write_end {
                    // The buffer in fp has been filled: one more byte, for
                    // a regular __overflow call (glibc bug 28949).
                    self.target = Target::One;
                    self.write_ptr = 0;
                    self.write_end = 1;
                }
                self.write_base = self.write_ptr;
            }
        }
    }

    // glibc: stdio-common/Xprintf_buffer_flush.c:Xprintf_buffer_flush
    fn flush<F: SysFile>(&mut self, fp: &mut Stream<F>) -> bool {
        if self.failed {
            return false;
        }
        self.flush_to_file(fp);
        !self.failed
    }

    // glibc: stdio-common/Xprintf_buffer_write.c:Xprintf_buffer_write
    fn write<F: SysFile>(&mut self, fp: &mut Stream<F>, mut s: &[u8]) {
        if self.failed {
            return;
        }
        while !s.is_empty() {
            if self.write_ptr == self.write_end && !self.flush(fp) {
                return;
            }
            let to_copy = (self.write_end - self.write_ptr).min(s.len());
            let dst = match self.target {
                Target::File => &mut fp.buf[self.write_ptr..self.write_ptr + to_copy],
                Target::Stage | Target::One => &mut fp.stage[self.write_ptr..self.write_ptr + to_copy],
            };
            dst.copy_from_slice(&s[..to_copy]);
            self.write_ptr += to_copy;
            s = &s[to_copy..];
        }
    }

    // glibc: stdio-common/printf_buffer_to_file.c:__printf_buffer_to_file_done,
    //        stdio-common/Xprintf_buffer_done.c:Xprintf_buffer_done
    fn done<F: SysFile>(mut self, fp: &mut Stream<F>) -> i32 {
        if self.failed {
            return -1;
        }
        self.flush_to_file(fp);
        if self.failed {
            return -1;
        }
        let total = self.written + (self.write_ptr - self.write_base);
        match i32::try_from(total) {
            Ok(n) => n,
            Err(_) => {
                fp.errno = EOVERFLOW;
                -1
            }
        }
    }
}

#[cfg(test)]
mod tests;
