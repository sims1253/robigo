//! robigo-rt's command line as `R`'s: `-f FILE`'s transcript (each line read
//! written after its prompt, as `Rstd_ReadConsole` does when R is not
//! interactive) and the start-up errors, against GNU R 4.6.2's. Each case's
//! expected output was recorded with `R` in the oracle image
//! (`scripts/oracle-run.sh 4.6.2`), with the same arguments, in the
//! environment the conformance suites use (`LANGUAGE=en LC_ALL=C.UTF-8
//! TZ=UTC R_DEFAULT_PACKAGES=NULL`). The corpora in echo mode check the
//! transcript over whole files; these cases check the reader's edges.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The input files, by name.
fn inputs() -> Vec<(&'static str, Vec<u8>)> {
    let long = format!("x <- c({})\nlength(x)\n", vec!["1"; 2100].join(", "));
    vec![
        // a statement per `;`, a continued one, a comment, a blank line
        (
            "echo.R",
            b"x <- 1; y <- 2\nx +\n  y\n# a comment\n\nf <- function() {\n  1\n}\nf()\nprint(\"a\"); 3\n".to_vec(),
        ),
        ("nonl.R", b"cat(\"no newline\")".to_vec()),
        ("crlf.R", b"x <- 1\r\nx\r\n".to_vec()),
        ("error.R", b"stop(\"boom\")\n1\n".to_vec()),
        // fgets reads at most CONSOLE_BUFFER_SIZE - 1 bytes of a line
        ("long.R", long.into_bytes()),
        // strlen ends the line read at a NUL
        ("nul.R", b"x <- 1\x00junk\n2\n".to_vec()),
        // an empty read, parsed all the same (PARSE_EOF)
        ("nul0.R", b"\x00junk\n1\n".to_vec()),
        // R_NoEcho, as the console reads it at each prompt
        ("opt.R", b"getOption(\"echo\")\noptions(echo = FALSE)\n1\noptions(echo = TRUE)\n2\n".to_vec()),
        ("optna.R", b"options(echo = NA)\n3\n".to_vec()),
        ("last.R", b".Last <- function() cat(\"bye\\n\")\n1\n".to_vec()),
        ("stdin.R", b"1 + 1\n".to_vec()),
    ]
}

/// What GNU R 4.6.2 does: (name, arguments, stdin, stdout, stderr, status).
type Case = (&'static str, Vec<&'static str>, Option<&'static str>, Vec<u8>, &'static [u8], i32);

fn cases(long_src: &[u8]) -> Vec<Case> {
    const ECHO: &[u8] = b"> x <- 1; y <- 2\n> x +\n+   y\n[1] 3\n> # a comment\n> \n> f <- function() {\n+   1\n+ }\n> f()\n[1] 1\n> print(\"a\"); 3\n[1] \"a\"\n[1] 3\n> \n";
    let q = |f: &'static str| vec!["--vanilla", "--quiet", "--no-save", "-f", f];
    // the first 4095 bytes, then the rest of the line after a continuation prompt
    let first = long_src.iter().position(|&b| b == b'\n').unwrap() + 1;
    let long = [b"> ", &long_src[..4095], b"+ ", &long_src[4095..first], b"> length(x)\n[1] 2100\n> \n"].concat();
    vec![
        ("echo", q("echo.R"), None, ECHO.to_vec(), b"", 0),
        ("nonl", q("nonl.R"), None, b"> cat(\"no newline\")\nno newline> \n".to_vec(), b"", 0),
        ("crlf", q("crlf.R"), None, b"> x <- 1\n> x\n[1] 1\n> \n".to_vec(), b"", 0),
        ("error", q("error.R"), None, b"> stop(\"boom\")\n".to_vec(), b"Error: boom\nExecution halted\n", 1),
        ("long", q("long.R"), None, long, b"", 0),
        ("nul", q("nul.R"), None, b"> x <- 1+ 2\n> \n".to_vec(), b"", 0),
        ("nul0", q("nul0.R"), None, b"> + 1\n[1] 1\n> \n".to_vec(), b"", 0),
        (
            "opt-f",
            q("opt.R"),
            None,
            b"> getOption(\"echo\")\n[1] TRUE\n> options(echo = FALSE)\n[1] 1\n> 2\n[1] 2\n> \n".to_vec(),
            b"",
            0,
        ),
        ("optna-f", q("optna.R"), None, b"> options(echo = NA)\n> 3\n[1] 3\n> \n".to_vec(), b"", 0),
        // Rscript: R_NoEcho until options(echo = TRUE)
        ("opt-rscript", vec!["--vanilla", "opt.R"], None, b"[1] FALSE\n[1] 1\n> 2\n[1] 2\n> \n".to_vec(), b"", 0),
        // end_Rmainloop's newline comes before .Last
        ("last", q("last.R"), None, b"> .Last <- function() cat(\"bye\\n\")\n> 1\n[1] 1\n> \nbye\n".to_vec(), b"", 0),
        (
            "no-echo",
            vec!["--vanilla", "--no-echo", "-f", "echo.R"],
            None,
            b"[1] 3\n[1] 1\n[1] \"a\"\n[1] 3\n".to_vec(),
            b"",
            0,
        ),
        ("file-eq", vec!["--vanilla", "--quiet", "--file=echo.R"], None, ECHO.to_vec(), b"", 0),
        (
            "file-stdin",
            vec!["--vanilla", "--quiet", "--file=-"],
            Some("stdin.R"),
            b"> 1 + 1\n[1] 2\n> \n".to_vec(),
            b"",
            0,
        ),
        // the R front end's check (R.sh.in)
        (
            "f-dash",
            vec!["--vanilla", "--quiet", "-f", "-"],
            Some("stdin.R"),
            Vec::new(),
            b"ERROR: option '-f' requires a filename argument\n",
            1,
        ),
        (
            "e-and-f",
            vec!["--vanilla", "--quiet", "-e", "1", "-f", "echo.R"],
            None,
            b"Fatal error: cannot use -e with -f or --file\n".to_vec(),
            b"",
            2,
        ),
        (
            "extra",
            vec!["--vanilla", "--quiet", "-f", "stdin.R", "extra"],
            None,
            b"ARGUMENT 'extra' __ignored__\n\n> 1 + 1\n[1] 2\n> \n".to_vec(),
            b"",
            0,
        ),
        (
            "args",
            vec!["--vanilla", "--quiet", "-f", "stdin.R", "--args", "a", "b"],
            None,
            b"> 1 + 1\n[1] 2\n> \n".to_vec(),
            b"",
            0,
        ),
        (
            "missing",
            q("nope.R"),
            None,
            b"Fatal error: cannot open file 'nope.R': No such file or directory\n".to_vec(),
            b"",
            2,
        ),
        // no FILE and no -e: stdin, not a terminal here, is the console
        // (`R --vanilla --no-echo < script`, as GNU R's own build runs R)
        (
            "stdin-no-echo",
            vec!["--vanilla", "--no-echo"],
            Some("echo.R"),
            b"[1] 3\n[1] 1\n[1] \"a\"\n[1] 3\n".to_vec(),
            b"",
            0,
        ),
        ("stdin-quiet", vec!["--vanilla", "--quiet"], Some("stdin.R"), b"> 1 + 1\n[1] 2\n> \n".to_vec(), b"", 0),
        // --args ends the options in any mode
        ("stdin-args", vec!["--vanilla", "--no-echo", "--args", "a"], Some("stdin.R"), b"[1] 2\n".to_vec(), b"", 0),
        // --no-echo is SA_NOSAVE too
        ("stdin-no-echo-save", vec!["--no-echo"], Some("stdin.R"), b"[1] 2\n".to_vec(), b"", 0),
        // stdin needs the save action named, a FILE implies it
        (
            "stdin-no-save",
            vec!["--quiet"],
            Some("stdin.R"),
            Vec::new(),
            b"Fatal error: you must specify '--save', '--no-save' or '--vanilla'\n",
            2,
        ),
        ("f-implies-no-save", vec!["--quiet", "-f", "stdin.R"], None, b"> 1 + 1\n[1] 2\n> \n".to_vec(), b"", 0),
        (
            "rscript-missing",
            vec!["nope.R"],
            None,
            b"Fatal error: cannot open file 'nope.R': No such file or directory\n".to_vec(),
            b"",
            2,
        ),
    ]
}

/// `robigo-rt ARGS` in `dir`: (stdout, stderr, status). The child inherits
/// the verification modes scripts/rt-ci.py runs the tests under (gctorture,
/// the moving collector, the observation modes, `ROBIGO_GC_STATS=check`'s
/// panics), so the cases hold under each; the reports those modes write to
/// stderr are not the program's and are left out.
fn run(dir: &Path, args: &[&str], stdin: Option<&str>) -> (Vec<u8>, Vec<u8>, i32) {
    run_env(dir, args, stdin, &[])
}

/// [`run`] with `env` set on top of its environment.
fn run_env(dir: &Path, args: &[&str], stdin: Option<&str>, env: &[(&str, &str)]) -> (Vec<u8>, Vec<u8>, i32) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_robigo-rt"));
    cmd.args(args).current_dir(dir).env("LANGUAGE", "en").env("LC_ALL", "C.UTF-8").env("TZ", "UTC");
    cmd.env("R_DEFAULT_PACKAGES", "NULL").env_remove("ROBIGO_FAIL_ALLOC_AT").env_remove("ROBIGO_BOOT_VERBOSE");
    // the pin report to stderr (`1`); to a file it stays, as rt-ci's check
    if std::env::var_os("ROBIGO_GC_PIN_REPORT").is_some_and(|v| v == "1") {
        cmd.env_remove("ROBIGO_GC_PIN_REPORT");
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("run robigo-rt");
    let input = stdin.map(|f| std::fs::read(dir.join(f)).unwrap()).unwrap_or_default();
    // from a thread, so a large input cannot block against a full stdout
    // pipe; a program that exits before reading all of it (an error, q())
    // leaves the rest unread, as GNU R does, and the transcript is the judge
    let mut pipe = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || match pipe.write_all(&input) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        r => r,
    });
    let o = child.wait_with_output().unwrap();
    writer.join().unwrap().expect("write robigo-rt's stdin");
    // ROBIGO_GC_STATS' report (the heap's, when the isolate is dropped)
    let err: Vec<u8> = o
        .stderr
        .split_inclusive(|&b| b == b'\n')
        .filter(|l| !l.starts_with(b"robigo gc: "))
        .flatten()
        .copied()
        .collect();
    (o.stdout, err, o.status.code().unwrap_or(-1))
}

#[test]
fn the_command_line_and_transcript_are_gnu_rs() {
    let dir: PathBuf = std::env::temp_dir().join(format!("robigo-rt-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let inputs = inputs();
    for (name, src) in &inputs {
        std::fs::write(dir.join(name), src).unwrap();
    }
    let long_src = &inputs.iter().find(|(n, _)| *n == "long.R").unwrap().1;
    let mut failed = Vec::new();
    for (name, args, stdin, out, err, status) in cases(long_src) {
        let got = run(&dir, &args, stdin);
        if got != (out.clone(), err.to_vec(), status) {
            failed.push(format!(
                "{name}: got\n  stdout {:?}\n  stderr {:?}\n  status {}\nwant\n  stdout {:?}\n  stderr {:?}\n  status {status}",
                String::from_utf8_lossy(&got.0),
                String::from_utf8_lossy(&got.1),
                got.2,
                String::from_utf8_lossy(&out),
                String::from_utf8_lossy(err),
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// R's start-up greeting (`PrintGreeting`, R without `--quiet`) is not
/// ported: robigo-rt says so rather than printing another.
/// A locale the C library does not have: setup_Rmainloop's deferred
/// warnings, one per category, "During startup - " (GNU R 4.6.2's, on the
/// oracle), and the C locale; quiet when neither LANG nor LC_ALL is set.
#[test]
fn a_missing_locale_warns_during_startup_and_uses_c() {
    let expr = "cat(l10n_info()[[4]], '\\n')";
    let (out, err, status) =
        run_env(&std::env::temp_dir(), &["--vanilla", "-e", expr], None, &[("LC_ALL", "xx_YY.UTF-8")]);
    let mut want = String::from("During startup - Warning messages:\n");
    for (i, c) in ["LC_CTYPE", "LC_COLLATE", "LC_TIME", "LC_MESSAGES", "LC_MONETARY", "LC_PAPER", "LC_MEASUREMENT"]
        .iter()
        .enumerate()
    {
        want.push_str(&format!("{}: Setting {c} failed, using \"C\" \n", i + 1));
    }
    assert_eq!(String::from_utf8_lossy(&err), want);
    assert_eq!((String::from_utf8_lossy(&out).as_ref(), status), ("ANSI_X3.4-1968 \n", 0));
}

#[test]
fn the_greeting_is_refused() {
    let dir: PathBuf = std::env::temp_dir().join(format!("robigo-rt-cli-greeting-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("one.R"), b"1\n").unwrap();
    let (out, err, status) = run(&dir, &["--vanilla", "--no-save", "-f", "one.R"], None);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!((out, status), (Vec::new(), 2));
    assert_eq!(
        String::from_utf8_lossy(&err),
        "robigo-rt: R's start-up greeting (R without --quiet) is not implemented yet\n"
    );
}

/// Without a save action, R on stdin stops at once: it does not wait for
/// the input to end (a pipe left open).
#[test]
fn the_save_check_does_not_wait_for_stdin() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_robigo-rt"));
    cmd.arg("--quiet").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("run robigo-rt");
    // (stdin stays open until the child has exited)
    let held = child.stdin.take();
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break Some(st);
        }
        if start.elapsed() > std::time::Duration::from_secs(20) {
            let _ = child.kill();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    drop(held);
    let out = child.wait_with_output().unwrap();
    assert_eq!(status.and_then(|s| s.code()), Some(2), "robigo-rt waited for stdin to end");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "Fatal error: you must specify '--save', '--no-save' or '--vanilla'\n"
    );
}

/// An empty `R_TRANSLATIONS` is GNU R 4.6.2's fatal start-up error
/// (main.c's BindDomain: `Rasprintf_malloc` of "" fails): "Fatal error:
/// allocation failure in BindDomain" on stdout, status 2 (R_Suicide).
/// `options(keep.source = TRUE)` would have the console keep source
/// references (gram.c's R_Parse1Buffer), which robigo-rt cannot yet: it is
/// refused, where GNU R accepts it (so no oracle case). `R_KEEP_PKG_SOURCE=yes`
/// starts `keep.source.pkgs` TRUE, as options.c's InitOptions does
/// (`keep.source` is Common.R's `interactive()`).
#[test]
fn keep_source_true_is_refused_and_keep_pkg_source_read() {
    let expr = "cat(tryCatch(options(keep.source = TRUE), error = conditionMessage), getOption('keep.source'), '\\n')";
    let (out, err, status) = run(&std::env::temp_dir(), &["--vanilla", "-e", expr], None);
    assert_eq!(
        String::from_utf8_lossy(&out),
        "robigo: options(keep.source = TRUE) (source references) is not implemented yet FALSE \n"
    );
    assert_eq!((String::from_utf8_lossy(&err).as_ref(), status), ("", 0));
    let expr = "cat(getOption('keep.source'), getOption('keep.source.pkgs'), '\\n')";
    for (v, want) in [("yes", "FALSE TRUE \n"), ("no", "FALSE FALSE \n"), ("YES", "FALSE FALSE \n")] {
        let (out, _, status) =
            run_env(&std::env::temp_dir(), &["--vanilla", "-e", expr], None, &[("R_KEEP_PKG_SOURCE", v)]);
        assert_eq!((String::from_utf8_lossy(&out).as_ref(), status), (want, 0), "R_KEEP_PKG_SOURCE={v}");
    }
}

#[test]
fn an_empty_r_translations_is_fatal() {
    let (out, err, status) = run_env(&std::env::temp_dir(), &["--vanilla", "-e", "1"], None, &[("R_TRANSLATIONS", "")]);
    assert_eq!(String::from_utf8_lossy(&out), "Fatal error: allocation failure in BindDomain\n");
    assert_eq!(String::from_utf8_lossy(&err), "");
    assert_eq!(status, 2);
}

/// With `_R_COMPILE_PKGS_` set, GNU R byte-compiles each closure as it is
/// serialized (serialize.c's WriteItem, `R_cmpfun1`); robigo-rt cannot yet,
/// so it refuses rather than write the closure uncompiled (AGENTS rule 6).
/// GNU R writes it, so there is no oracle case to pass.
#[test]
fn compiling_closures_on_serialize_is_refused() {
    let expr = "x <- tryCatch(serialize(function(a) a, NULL), error = conditionMessage); cat(is.raw(x), if (is.character(x)) x, '\\n')";
    let (out, err, status) = run_env(
        &std::env::temp_dir(),
        &["--vanilla", "-e", expr],
        None,
        &[("_R_COMPILE_PKGS_", "1"), ("R_ENABLE_JIT", "0")],
    );
    assert_eq!(
        String::from_utf8_lossy(&out),
        "FALSE robigo: byte-compiling closures as they are serialized (_R_COMPILE_PKGS_) is not implemented yet \n"
    );
    assert_eq!((String::from_utf8_lossy(&err).as_ref(), status), ("", 0));
    // a value with no closure in it is written as before, and so is any
    // closure when compiling packages is off
    let expr = "cat(is.raw(serialize(list(1, 'a'), NULL)), '\\n')";
    let (out, _, status) = run_env(
        &std::env::temp_dir(),
        &["--vanilla", "-e", expr],
        None,
        &[("_R_COMPILE_PKGS_", "1"), ("R_ENABLE_JIT", "0")],
    );
    assert_eq!((String::from_utf8_lossy(&out).as_ref(), status), ("TRUE \n", 0));
    let expr = "cat(is.raw(serialize(function(a) a, NULL)), '\\n')";
    let (out, _, status) = run_env(&std::env::temp_dir(), &["--vanilla", "-e", expr], None, &[("R_ENABLE_JIT", "0")]);
    assert_eq!((String::from_utf8_lossy(&out).as_ref(), status), ("TRUE \n", 0));
}
