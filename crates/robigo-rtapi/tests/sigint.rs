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
    cmd.env("R_DEFAULT_PACKAGES", "NULL").env_remove("ROBIGO_FAIL_ALLOC_AT").env_remove("ROBIGO_BOOT_VERBOSE");
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
