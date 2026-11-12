//! A script read from stdin (`--file=-`) runs through the console REPL
//! (R: main.c:Rf_ReplIteration), which echoes each line unless
//! `--no-echo`. The expected bytes are GNU R 4.5.0's (`R --vanilla --quiet
//! [--no-echo] --file=-`, C.UTF-8).

use std::io::Write;
use std::process::{Command, Stdio};

/// As in conformance.rs: `ROBIGO_CONF_BIN` tests the binary CI ships.
fn robigo_bin() -> std::path::PathBuf {
    std::env::var_os("ROBIGO_CONF_BIN").map_or_else(|| env!("CARGO_BIN_EXE_robigo").into(), Into::into)
}

fn run(input: &[u8], echo: bool) -> (Option<i32>, Vec<u8>, String) {
    let mut args = vec!["--vanilla", "--quiet"];
    if !echo {
        args.push("--no-echo");
    }
    args.push("--file=-");
    let mut child = Command::new(robigo_bin())
        .args(&args)
        .env("R_DEFAULT_PACKAGES", "NULL")
        .env("LANGUAGE", "en")
        .env("LC_ALL", "C.UTF-8")
        .env("TZ", "UTC")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn robigo");
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.code(), out.stdout, String::from_utf8_lossy(&out.stderr).into_owned())
}

const HALTED: &str = "Error: invalid multibyte character in parser (<input>:1:6)\nExecution halted\n";

#[test]
fn invalid_byte_in_stdin_script_halts() {
    // 0xFF announces a six-byte character, " cat(", which is invalid. The
    // echo shows the line as read; the error stops R before
    // end_Rmainloop, so no trailing newline follows it.
    assert_eq!(run(b"\xff cat(1)\n", true), (Some(1), b"> \xff cat(1)\n".to_vec(), HALTED.into()));
    assert_eq!(run(b"\xff cat(1)\n", false), (Some(1), Vec::new(), HALTED.into()));
}

#[test]
fn stdin_script_end_prints_trailing_newline_when_echoing() {
    // end_Rmainloop: the end of input after the last prompt is a newline in
    // echo mode, nothing with --no-echo.
    assert_eq!(run(b"cat(1)\n", true), (Some(0), b"> cat(1)\n1> \n".to_vec(), String::new()));
    assert_eq!(run(b"cat(1)\n", false), (Some(0), b"1".to_vec(), String::new()));
    // an invalid byte in a comment is never decoded
    assert_eq!(run(b"# \xff\ncat(1)\n", true), (Some(0), b"> # \xff\n> cat(1)\n1> \n".to_vec(), String::new()));
}
