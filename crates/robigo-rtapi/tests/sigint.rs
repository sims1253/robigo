//! A user interrupt from outside the process: SIGINT, which robigo-rt
//! takes at its poll points as GNU R does (main.c:handleInterrupt,
//! errors.c:onintrEx). Each case's script writes a marker line, then loops;
//! the test waits for each marker and sends SIGINT once per marker. The
//! expected output was recorded with GNU R 4.6.2 in the oracle image the
//! same way (`R --vanilla --no-echo -f FILE`, SIGINT reset to its default
//! before the exec, since a shell starts a background job with it ignored,
//! and GNU R keeps an ignored SIGINT ignored).

#![allow(unsafe_code)]

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// (name, script, markers to interrupt at, stdout, stderr, status).
type Case = (&'static str, &'static str, usize, &'static [u8], &'static [u8], i32);

const CASES: &[Case] = &[
    // uncaught: the console's newline, then the session halts
    ("loop", "cat(\"ready\\n\")\nrepeat {}\n", 1, b"ready\n", b"\nExecution halted\n", 1),
    // an exiting handler takes the first; the second unwinds f's on.exit
    (
        "caught",
        "r <- tryCatch({\n    cat(\"ready\\n\")\n    repeat {}\n}, interrupt = function(c) \"caught\")\nprint(r)\n\
         f <- function() { on.exit(cat(\"on.exit\\n\")); repeat {} }\ncat(\"again\\n\")\nf()\ncat(\"not reached\\n\")\n",
        2,
        b"ready\n[1] \"caught\"\nagain\non.exit\n",
        b"\nExecution halted\n",
        1,
    ),
];

/// Run `script`, sending SIGINT after each of the first `markers` marker
/// lines (`ready`, `again`) of its output: (stdout, stderr, status).
fn run(name: &str, script: &str, markers: usize) -> (Vec<u8>, Vec<u8>, i32) {
    let dir: PathBuf = std::env::temp_dir().join(format!("robigo-rt-sigint-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{name}.R"));
    std::fs::write(&file, script).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_robigo-rt"));
    cmd.arg(&file).env("LANGUAGE", "en").env("LC_ALL", "C.UTF-8").env("TZ", "UTC");
    // (an inherited R_HOME, GNU R's, robigo-rt ignores with a warning on
    // stdout)
    cmd.env("R_DEFAULT_PACKAGES", "NULL")
        .env_remove("R_HOME")
        .env_remove("ROBIGO_RT_CHECKOUT")
        .env_remove("ROBIGO_FAIL_ALLOC_AT")
        .env_remove("ROBIGO_BOOT_VERBOSE");
    // the pin report to stderr (`1`); to a file it stays, as rt-ci's check
    // (the verification modes run these cases too, as cli.rs's)
    if std::env::var_os("ROBIGO_GC_PIN_REPORT").is_some_and(|v| v == "1") {
        cmd.env_remove("ROBIGO_GC_PIN_REPORT");
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    // SAFETY: signal() is async-signal-safe, the one call in the child
    // between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("robigo-rt runs");
    let pid = child.id() as libc::pid_t;
    let out = Arc::new(Mutex::new(Vec::new()));
    let mut so = child.stdout.take().unwrap();
    let o = Arc::clone(&out);
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = so.read(&mut buf) {
            if n == 0 {
                break;
            }
            o.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    let mut se = child.stderr.take().unwrap();
    let err_reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        se.read_to_end(&mut v).unwrap();
        v
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    for k in 1..=markers {
        loop {
            let seen = String::from_utf8_lossy(&out.lock().unwrap())
                .lines()
                .filter(|l| *l == "ready" || *l == "again")
                .count();
            if seen >= k {
                break;
            }
            assert!(Instant::now() < deadline, "{name}: no marker {k} in time");
            std::thread::sleep(Duration::from_millis(20));
        }
        // the loop runs a moment past the marker, as in the recording
        std::thread::sleep(Duration::from_millis(200));
        // SAFETY: kill() of our own child process.
        assert_eq!(unsafe { libc::kill(pid, libc::SIGINT) }, 0);
    }
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("{name}: robigo-rt did not end after SIGINT");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    reader.join().unwrap();
    // ROBIGO_GC_STATS' report (the heap's, when the isolate is dropped)
    let err: Vec<u8> = err_reader
        .join()
        .unwrap()
        .split_inclusive(|&b| b == b'\n')
        .filter(|l| !l.starts_with(b"robigo gc: "))
        .flatten()
        .copied()
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    let out = out.lock().unwrap().clone();
    (out, err, status.code().unwrap_or(-1))
}

#[test]
fn sigint_is_gnu_rs_user_interrupt() {
    for &(name, script, markers, out, err, status) in CASES {
        let (o, e, s) = run(name, script, markers);
        assert_eq!(
            (String::from_utf8_lossy(&o), String::from_utf8_lossy(&e), s),
            (String::from_utf8_lossy(out), String::from_utf8_lossy(err), status),
            "{name}"
        );
    }
}

/// A user interrupt while the interactive console waits for input
/// (`R --no-readline --interactive`): sys-std.c's `handleInterrupt`,
/// `onintrNoResume`: the console's newline, the statement begun dropped, a
/// new prompt. Recorded with GNU R 4.6.2 in the oracle image reading a FIFO
/// the same way (SIGINT reset to its default): `x <- 1 +`, SIGINT at the
/// continuation prompt, then `2` and `x <- 5; x`.
#[test]
fn an_interrupt_at_the_interactive_prompt_starts_a_new_statement() {
    let want = ("> + > [1] 2\n> [1] 5\n> \n", "\n", Some(0));
    // GNU R's transcript under M1's loader, which runs no profile (the
    // console's test, ratcheted)
    let (out, err, status) = interrupt_session("m1");
    assert_eq!((out.as_str(), err.as_str(), status), want);
    // INTERIM(srcref): under the default boot the system profile's
    // options(keep.source = interactive()) is refused first, once (as
    // tests/cli.rs's SRCREF cases); when source references exist this fails
    // and the run above takes the default boot
    let (out, err, status) = interrupt_session("");
    let refusal =
        "Error in options(...) : \n  robigo: options(keep.source = TRUE) (source references) is not implemented yet\n";
    assert_eq!((out.as_str(), err.as_str(), status), (want.0, format!("{refusal}{}", want.1).as_str(), want.2));
}

/// [`an_interrupt_at_the_interactive_prompt_starts_a_new_statement`]'s
/// session under `boot` (ROBIGO_RT_BOOT): (stdout, stderr, status).
fn interrupt_session(boot: &str) -> (String, String, Option<i32>) {
    use std::io::Write;
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_robigo-rt"));
    cmd.args(["--no-readline", "--interactive", "--no-save", "--quiet"])
        .env("ROBIGO_RT_BOOT", boot)
        .env("LANGUAGE", "en")
        .env("LC_ALL", "C.UTF-8")
        .env("TZ", "UTC")
        .env("R_DEFAULT_PACKAGES", "NULL")
        .env_remove("R_HOME")
        .env_remove("ROBIGO_RT_CHECKOUT")
        .env_remove("ROBIGO_FAIL_ALLOC_AT")
        .env_remove("ROBIGO_BOOT_VERBOSE");
    if std::env::var_os("ROBIGO_GC_PIN_REPORT").is_some_and(|v| v == "1") {
        cmd.env_remove("ROBIGO_GC_PIN_REPORT");
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::piped());
    // SAFETY: signal() is async-signal-safe, the one call in the child
    // between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("robigo-rt runs");
    let pid = child.id() as libc::pid_t;
    let out = Arc::new(Mutex::new(Vec::new()));
    let mut so = child.stdout.take().unwrap();
    let o = Arc::clone(&out);
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = so.read(&mut buf) {
            if n == 0 {
                break;
            }
            o.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    let mut se = child.stderr.take().unwrap();
    let err_reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        se.read_to_end(&mut v).unwrap();
        v
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let wait_for = |want: &[u8]| loop {
        if out.lock().unwrap().ends_with(want) {
            break;
        }
        assert!(Instant::now() < deadline, "no {:?} in time", String::from_utf8_lossy(want));
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"x <- 1 +\n").unwrap();
    stdin.flush().unwrap();
    wait_for(b"> + ");
    std::thread::sleep(Duration::from_millis(200));
    // SAFETY: kill() of our own child process.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGINT) }, 0);
    wait_for(b"> + > ");
    stdin.write_all(b"2\nx <- 5; x\n").unwrap();
    drop(stdin);
    // bounded: a console that hangs after the interrupt fails the test,
    // with what it wrote, instead of holding the job to its timeout
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "robigo-rt did not end after the console's input; it wrote {:?}",
                String::from_utf8_lossy(&out.lock().unwrap())
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    reader.join().unwrap();
    let err: Vec<u8> = err_reader
        .join()
        .unwrap()
        .split_inclusive(|&b| b == b'\n')
        .filter(|l| !l.starts_with(b"robigo gc: "))
        .flatten()
        .copied()
        .collect();
    let out = out.lock().unwrap().clone();
    (String::from_utf8_lossy(&out).into_owned(), String::from_utf8_lossy(&err).into_owned(), status.code())
}
