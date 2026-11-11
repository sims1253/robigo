//! R's console input: `R_ReadConsole` and the `stdin()` connection that
//! reads through it.
//!
//! `R -f FILE`, `Rscript FILE` and `R -e EXPR` make a file the console
//! (R: unix/system.c's `ifp`); otherwise it is the process's standard
//! input. The REPL and every console reader (`readLines()`, `scan()`,
//! `readline()` at a terminal) take their lines from this one stream, so a
//! script can read the lines that follow the statement it is running, as
//! with GNU R.
//!
//! Ports:
//! - `R: unix/sys-std.c:Rstd_ReadConsole`: the prompt, the echo of each
//!   line exactly as read, CRLF read as LF, the newline appended to a final
//!   line without one, reads capped at `CONSOLE_BUFFER_SIZE` bytes.
//! - `R: main/connections.c:ConsoleGetchar` / `stdin_fgetc`: the `stdin()`
//!   connection, a character at a time out of its own console buffer,
//!   refilled by `R_ReadConsole("")`.
//! - `R: main/scan.c:ConsoleGetchar` and `ConsolePrompt`: `scan(file = "")`
//!   and an interactive `readline()`, out of a second buffer refilled with
//!   their prompt (`1: `, `2: `, ...).

use std::cell::RefCell;
use std::io::{Read, Write};

/// `CONSOLE_BUFFER_SIZE` (R: src/include/Defn.h): reads return at most
/// this many bytes (including the terminating NUL in C), so longer lines
/// are read (and echoed) in pieces.
pub const CONSOLE_BUFFER_SIZE: usize = 4096;

/// R's `R_EOF`.
const EOF: i32 = -1;

/// `CONSOLE_PROMPT_SIZE` (R: src/include/Defn.h), including the NUL.
const CONSOLE_PROMPT_SIZE: usize = 256;

/// A console buffer that a reader takes a byte at a time, refilled one
/// `R_ReadConsole` at a time (`ConsoleBuf`, `ConsoleBufp`).
#[derive(Default)]
struct LineBuf {
    line: Vec<u8>,
    pos: usize,
}

struct Console {
    /// `ifp ? ifp : stdin`: read a byte at a time, so it should buffer.
    input: Box<dyn Read>,
    /// One byte of lookahead: `feof` after `fgets`, which the final-newline
    /// rule depends on.
    pending: Option<u8>,
    /// `R_Interactive`: the line editor shows the prompt and the terminal
    /// echoes what is typed.
    interactive: bool,
    /// `!R_NoEcho`: a non-interactive console prints the prompt and echoes
    /// each line it reads.
    echo: bool,
    /// connections.c's `ConsoleBuf`.
    conbuf: LineBuf,
    /// scan.c's `ConsoleBuf` and `ConsolePrompt`.
    scanbuf: LineBuf,
    prompt: String,
}

impl Console {
    fn getc(&mut self) -> Option<u8> {
        if let Some(b) = self.pending.take() {
            return Some(b);
        }
        let mut byte = [0u8; 1];
        loop {
            match self.input.read(&mut byte) {
                Ok(0) => return None,
                Ok(_) => return Some(byte[0]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }
    }

    // R: unix/sys-std.c:Rstd_ReadConsole
    fn read_console(&mut self, prompt: &str) -> Option<Vec<u8>> {
        if self.interactive || self.echo {
            out(prompt.as_bytes());
        }
        // `fgets(buf, len, ifp ? ifp : stdin)`: up to len - 1 bytes,
        // stopping after a newline.
        let mut bytes: Vec<u8> = Vec::new();
        while bytes.len() < CONSOLE_BUFFER_SIZE - 1 {
            let Some(b) = self.getc() else { break };
            bytes.push(b);
            if b == b'\n' {
                break;
            }
        }
        if bytes.is_empty() {
            return None;
        }
        // Remove CR in CRLF ending.
        if bytes.ends_with(b"\r\n") {
            let n = bytes.len();
            bytes.truncate(n - 2);
            bytes.push(b'\n');
        }
        // At end of input, a line not terminated by `\n` gets one (so the
        // transcript ends with a newline).
        if !bytes.ends_with(b"\n") {
            match self.getc() {
                Some(b) => self.pending = Some(b),
                None => bytes.push(b'\n'),
            }
        }
        if !self.interactive && self.echo {
            out(&bytes);
        }
        Some(bytes)
    }

    /// The next byte of `buf`, read through `R_ReadConsole(prompt)` when it
    /// is used up (R: connections.c:ConsoleGetchar and scan.c:ConsoleGetchar,
    /// which differ only in buffer and prompt).
    fn buffered_getchar(&mut self, scan: bool) -> i32 {
        let buf = if scan { &self.scanbuf } else { &self.conbuf };
        if buf.pos >= buf.line.len() {
            let prompt = if scan { self.prompt.clone() } else { String::new() };
            let line = self.read_console(&prompt);
            let buf = if scan { &mut self.scanbuf } else { &mut self.conbuf };
            buf.pos = 0;
            match line {
                Some(line) => buf.line = line,
                None => {
                    buf.line.clear();
                    return EOF;
                }
            }
        }
        let buf = if scan { &mut self.scanbuf } else { &mut self.conbuf };
        buf.pos += 1;
        buf.line[buf.pos - 1] as i32
    }
}

thread_local! {
    static CONSOLE: RefCell<Console> = RefCell::new(Console {
        input: Box::new(std::io::stdin()),
        pending: None,
        interactive: false,
        echo: false,
        conbuf: LineBuf::default(),
        scanbuf: LineBuf::default(),
        prompt: String::new(),
    });
}

/// Prompts and echo go straight to the process's standard output, as R's
/// `fputs(..., stdout)` does: past `sink()`, but in order with the
/// interpreter's own output, which writes to the same `Stdout`.
fn out(s: &[u8]) {
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(s);
    let _ = stdout.flush();
}

/// Make `input` the console (R: unix/system.c sets `ifp` for `-f` and
/// `-e`; otherwise the console is standard input). `interactive` is
/// `R_Interactive`, `echo` is `!R_NoEcho`.
pub fn set_input(input: Box<dyn Read>, interactive: bool, echo: bool) {
    CONSOLE.with(|c| {
        let mut c = c.borrow_mut();
        c.input = input;
        c.pending = None;
        c.interactive = interactive;
        c.echo = echo;
        c.conbuf = LineBuf::default();
        c.scanbuf = LineBuf::default();
    });
}

/// Set how the console shows prompts and input, keeping its stream.
pub fn set_mode(interactive: bool, echo: bool) {
    CONSOLE.with(|c| {
        let mut c = c.borrow_mut();
        c.interactive = interactive;
        c.echo = echo;
    });
}

/// `R_ReadConsole(prompt, buf, CONSOLE_BUFFER_SIZE, ...)`: the next line
/// of console input (at most `CONSOLE_BUFFER_SIZE - 1` bytes, ending in a
/// newline unless the line was longer), or `None` at end of input.
pub fn read_console(prompt: &str) -> Option<Vec<u8>> {
    CONSOLE.with(|c| c.borrow_mut().read_console(prompt))
}

/// The `stdin()` connection's `fgetc` (R: connections.c:stdin_fgetc): the
/// next byte of console input, or -1 at end of input.
pub fn getchar() -> i32 {
    CONSOLE.with(|c| c.borrow_mut().buffered_getchar(false))
}

/// scan.c's `ConsoleGetchar`: the next byte of console input for
/// `scan(file = "")` and `readline()`, prompting with [`prompt`] when a
/// line is needed; -1 at end of input.
pub fn prompt_getchar() -> i32 {
    CONSOLE.with(|c| c.borrow_mut().buffered_getchar(true))
}

/// scan.c's `ConsolePrompt`, cut to `CONSOLE_PROMPT_SIZE - 1` bytes like
/// its `strncpy`.
pub fn set_prompt(prompt: &str) {
    let mut end = prompt.len().min(CONSOLE_PROMPT_SIZE - 1);
    while !prompt.is_char_boundary(end) {
        end -= 1;
    }
    CONSOLE.with(|c| c.borrow_mut().prompt = prompt[..end].to_string());
}

/// The current `ConsolePrompt`.
pub fn prompt() -> String {
    CONSOLE.with(|c| c.borrow().prompt.clone())
}

/// Read up to `buf.len()` bytes of console input for the `stdin()`
/// connection's binary reads, line by line through the console buffer.
pub fn read(buf: &mut [u8]) -> usize {
    let mut n = 0;
    while n < buf.len() {
        let c = getchar();
        if c == EOF {
            break;
        }
        buf[n] = c as u8;
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn console(input: &str) -> Console {
        Console {
            input: Box::new(std::io::Cursor::new(input.as_bytes().to_vec())),
            pending: None,
            interactive: false,
            echo: false,
            conbuf: LineBuf::default(),
            scanbuf: LineBuf::default(),
            prompt: String::new(),
        }
    }

    #[test]
    fn lines_and_final_newline() {
        let mut c = console("a\r\nb");
        assert_eq!(c.read_console("> ").as_deref(), Some(&b"a\n"[..]));
        assert_eq!(c.read_console("> ").as_deref(), Some(&b"b\n"[..]));
        assert_eq!(c.read_console("> "), None);
    }

    #[test]
    fn long_lines_come_in_pieces() {
        let long = "x".repeat(CONSOLE_BUFFER_SIZE + 10) + "\n";
        let mut c = console(&long);
        assert_eq!(c.read_console("").map(|l| l.len()), Some(CONSOLE_BUFFER_SIZE - 1));
        assert_eq!(c.read_console("").map(|l| l.len()), Some(12));
    }

    #[test]
    fn getchar_shares_the_stream() {
        let mut c = console("one\ntwo\nthree\n");
        assert_eq!(c.read_console("> ").as_deref(), Some(&b"one\n"[..]));
        let got: Vec<u8> = (0..4).map(|_| c.buffered_getchar(false) as u8).collect();
        assert_eq!(got, b"two\n");
        assert_eq!(c.read_console("> ").as_deref(), Some(&b"three\n"[..]));
        assert_eq!(c.buffered_getchar(false), EOF);
    }

    #[test]
    fn scan_and_stdin_buffers_are_separate() {
        let mut c = console("ab\ncd\n");
        assert_eq!(c.buffered_getchar(true), b'a' as i32);
        assert_eq!(c.buffered_getchar(false), b'c' as i32);
        assert_eq!(c.buffered_getchar(true), b'b' as i32);
    }
}
