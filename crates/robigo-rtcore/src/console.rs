//! The isolate's console (docs/g3.md §11): where GNU R's `Rprintf` and
//! `REprintf` write. Per isolate, so two sessions in one process print to
//! their own streams; the embedder (the `robigo-rt` binary, a test, A1's
//! C API) chooses them with [`Cx::replace_console`].
//!
//! GNU R writes `Rprintf` through `R_WriteConsoleEx(buf, len, 0)` (Rscript:
//! the C library's buffered `stdout`) and `REprintf` to `stderr`
//! unbuffered. Here an `REprintf` flushes the output stream first, so a
//! terminal or a merged pipe sees the order the program wrote in; the two
//! streams' contents are GNU R's either way. Write errors are ignored, as
//! GNU R ignores them.

use crate::isolate::Cx;
use std::io::Write;

/// Hook called by [`Cx::rprintf`] or [`Cx::reprintf`] while an output or
/// message sink is active (`sink()`).
///
/// Return `true` when the hook has delivered `b` and the console write should
/// be suppressed, or `false` to fall through to the console. A hook that
/// forwards or splits bytes back to the console must call
/// [`Cx::write_console_out`] or [`Cx::write_console_err`] (never
/// [`Cx::rprintf`] or [`Cx::reprintf`], which would recurse into the hook).
/// The hook always runs with no console `RefCell` borrow held, so calling
/// `write_console_out` / `write_console_err` from inside it is safe.
pub type SinkHook = fn(&Cx, &[u8]) -> bool;

/// The two streams of a console.
pub struct Console {
    out: Box<dyn Write>,
    err: Box<dyn Write>,
}

impl Console {
    /// A console writing to `out` (`Rprintf`) and `err` (`REprintf`).
    pub fn new(out: Box<dyn Write>, err: Box<dyn Write>) -> Console {
        Console { out, err }
    }

    /// The process's standard streams, the output buffered (Rscript's).
    pub fn std() -> Console {
        Console::new(Box::new(std::io::BufWriter::new(std::io::stdout())), Box::new(std::io::stderr()))
    }
}

/// A `Write` into a shared buffer: a console that tests and embedders
/// read back ([`Console::capture`]).
#[derive(Clone, Default)]
pub struct SharedBuf(pub std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

impl SharedBuf {
    /// What was written, taken out.
    pub fn take(&self) -> Vec<u8> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

impl Write for SharedBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Console {
    /// A console writing into two buffers, returned with it.
    pub fn capture() -> (Console, SharedBuf, SharedBuf) {
        let (out, err) = (SharedBuf::default(), SharedBuf::default());
        (Console::new(Box::new(out.clone()), Box::new(err.clone())), out, err)
    }
}

impl Default for Console {
    fn default() -> Console {
        Console::std()
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.out.flush();
        let _ = self.err.flush();
    }
}

impl Cx {
    /// `Rprintf`: `b` written to the console's output (or the active output
    /// sink when `sink()` has diverted it).
    // R: printutils.c:Rprintf, Rvprintf
    pub fn rprintf(&self, b: &[u8]) {
        if let Some(f) = self.iso().out_sink.get()
            && f(self, b)
        {
            return;
        }
        self.write_console_out(b);
    }

    /// `REprintf`: `b` written to the console's error stream (after the
    /// output is flushed), or to the active message sink when
    /// `sink(..., type = "message")` has diverted it.
    // R: printutils.c:REprintf, REvprintf
    pub fn reprintf(&self, b: &[u8]) {
        if let Some(f) = self.iso().err_sink.get()
            && f(self, b)
        {
            return;
        }
        self.write_console_err(b);
    }

    /// Write directly to the console's output stream, bypassing `out_sink`.
    pub fn write_console_out(&self, b: &[u8]) {
        let mut c = self.iso().console.borrow_mut();
        let _ = c.out.write_all(b);
    }

    /// Write directly to the console's error stream (flushing output first),
    /// bypassing `err_sink`.
    pub fn write_console_err(&self, b: &[u8]) {
        let mut c = self.iso().console.borrow_mut();
        let _ = c.out.flush();
        let _ = c.err.write_all(b);
        let _ = c.err.flush();
    }

    /// Install or clear the output-sink hook (`sink(type = "output")`).
    /// See [`SinkHook`] for the return-value and re-entrancy contract.
    pub fn set_out_sink(&self, f: Option<SinkHook>) {
        self.iso().out_sink.set(f);
    }

    /// Install or clear the message-sink hook (`sink(type = "message")`).
    /// See [`SinkHook`] for the return-value and re-entrancy contract.
    pub fn set_err_sink(&self, f: Option<SinkHook>) {
        self.iso().err_sink.set(f);
    }

    /// `R_FlushConsole`: the output stream flushed.
    pub fn flush_console(&self) {
        let _ = self.iso().console.borrow_mut().out.flush();
    }

    /// Make `c` the console; the previous one returned (flushed).
    pub fn replace_console(&self, c: Console) -> Console {
        let mut old = self.iso().console.replace(c);
        let _ = old.out.flush();
        old
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OwnedIsolate;

    #[test]
    fn writes_to_the_isolates_console() {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        let (c, out, err) = Console::capture();
        cx.replace_console(c);
        cx.rprintf(b"[1] 1\n");
        cx.reprintf(b"Error: x\n");
        assert_eq!(out.take(), b"[1] 1\n");
        assert_eq!(err.take(), b"Error: x\n");
        // a second isolate has its own
        let mut other = OwnedIsolate::new();
        let (c2, o2, _) = Console::capture();
        other.cx().replace_console(c2);
        other.cx().rprintf(b"two");
        cx.rprintf(b"one");
        assert_eq!(out.take(), b"one");
        assert_eq!(o2.take(), b"two");
    }

    #[test]
    fn sink_hooks_consume_fall_through_and_allow_console_reentry() {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        let (c, out, err) = Console::capture();
        cx.replace_console(c);

        // Consuming hook suppresses console write.
        cx.set_out_sink(Some(|_cx, _b| true));
        cx.rprintf(b"suppressed");
        assert_eq!(out.take(), b"");

        // Non-consuming hook falls through to the console.
        cx.set_out_sink(Some(|_cx, _b| false));
        cx.rprintf(b"kept");
        assert_eq!(out.take(), b"kept");

        // Hook can write directly to the console without panicking on a borrow
        // (e.g. split = TRUE or a sink pointing at stdout).
        cx.set_out_sink(Some(|cx, b| {
            cx.write_console_out(b"[");
            cx.write_console_out(b);
            cx.write_console_out(b"]");
            true
        }));
        cx.rprintf(b"split");
        assert_eq!(out.take(), b"[split]");

        // Clearing restores direct writes.
        cx.set_out_sink(None);
        cx.rprintf(b"plain");
        assert_eq!(out.take(), b"plain");

        // Same contract for the message (stderr) sink.
        cx.set_err_sink(Some(|_cx, _b| true));
        cx.reprintf(b"suppressed-err");
        assert_eq!(err.take(), b"");

        cx.set_err_sink(Some(|_cx, _b| false));
        cx.reprintf(b"kept-err");
        assert_eq!(err.take(), b"kept-err");

        cx.set_err_sink(Some(|cx, b| {
            cx.write_console_err(b"<");
            cx.write_console_err(b);
            cx.write_console_err(b">");
            true
        }));
        cx.reprintf(b"reenter-err");
        assert_eq!(err.take(), b"<reenter-err>");

        cx.set_err_sink(None);
        cx.reprintf(b"plain-err");
        assert_eq!(err.take(), b"plain-err");
    }
}
