//! L5's boot of base from its database (`ROBIGO_RT_BOOT=rdb`): GNU R's
//! `R_ReplFile` of R_HOME's `library/base/R/base`, which in the tree
//! `scripts/rt-home-db.sh` makes is `baseloader.R`, its `..lazyLoad`
//! binding base from the `base.rdb` robigo-rt wrote (makebasedb.R, booted
//! from base's source). Checked: base's closures are byte code (from the
//! database, compiled as they were written), baseloader.R's work after
//! `..lazyLoad` (the `.C_`/`.F_` symbols, the S3 methods table, the args
//! environments locked), `InitGraphics`'s `.Device`, and base's names: as
//! many as GNU R 4.6.2's (`ls(baseenv(), all.names = TRUE)` with
//! `R_DEFAULT_PACKAGES=NULL`: 1407).
//!
//! The tree is not in the repository: `ROBIGO_RT_HOME_DB=1` (rt-ci's step,
//! after `scripts/rt-home-db.sh`) says it must be there. Without the
//! variable the test says so and passes when the tree is missing.

use std::process::Command;

/// What base's boot gives on stderr: nothing. The system profile runs to
/// its end (its `Sys.info` and `Sys.glob` calls, through
/// `.expand_R_libs_env_var` and `.libPaths`, are builtins now).
const PROFILE_STOPS: &str = "";

/// Whether the tree is made by this checkout's scripts/rt-home-db.sh (else
/// the test is skipped, unless `ROBIGO_RT_HOME_DB` says it must be there):
/// the script's hash is its stamp's second field from the end, so a tree an
/// older script made is stale and says so, rather than failing on what it
/// lacks (an older tree has no etc/Renviron).
fn tree_made() -> bool {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tree = root.join(".robigo/rt-home-db");
    let must = std::env::var_os("ROBIGO_RT_HOME_DB").is_some();
    if !tree.join("library/base/R/base.rdb").is_file() {
        assert!(
            !must,
            "ROBIGO_RT_HOME_DB is set, but {} has no base.rdb: make it with scripts/rt-home-db.sh",
            tree.display()
        );
        eprintln!("{} is not made: skipped (scripts/rt-home-db.sh makes it)", tree.display());
        return false;
    }
    let script = root.join("scripts/rt-home-db.sh");
    let o = Command::new("sha256sum").arg(&script).output().unwrap_or_else(|e| panic!("running sha256sum: {e}"));
    let want = String::from_utf8_lossy(&o.stdout).chars().take(16).collect::<String>();
    let stamp = std::fs::read_to_string(tree.join(".rt-home-db")).unwrap_or_default();
    if stamp.split_whitespace().rev().nth(1) != Some(want.as_str()) {
        assert!(
            !must,
            "{} is stale (made by another scripts/rt-home-db.sh): rebuild it with scripts/rt-home-db.sh --robigo-rt BIN",
            tree.display()
        );
        eprintln!("{} is stale: skipped (rebuild it with scripts/rt-home-db.sh --robigo-rt BIN)", tree.display());
        return false;
    }
    true
}

/// robigo-rt booted from the tree with `args` and the extra environment
/// `env`: (stdout, stderr, status).
fn boot(args: &[&std::ffi::OsStr], env: &[(&str, &std::ffi::OsStr)]) -> (String, String, Option<i32>) {
    boot_bin_reading(std::path::Path::new(env!("CARGO_BIN_EXE_robigo-rt")), args, env, &[], None)
}

/// [`boot`] with the variables `remove` removed after the rest (the
/// boot's own `R_DEFAULT_PACKAGES=NULL` among them).
fn boot_without(
    args: &[&std::ffi::OsStr],
    env: &[(&str, &std::ffi::OsStr)],
    remove: &[&str],
) -> (String, String, Option<i32>) {
    boot_bin_reading(std::path::Path::new(env!("CARGO_BIN_EXE_robigo-rt")), args, env, remove, None)
}

/// [`boot`] with `stdin` as the console's input (None: no stdin).
fn boot_reading(
    args: &[&std::ffi::OsStr],
    env: &[(&str, &std::ffi::OsStr)],
    stdin: Option<&[u8]>,
) -> (String, String, Option<i32>) {
    boot_bin_reading(std::path::Path::new(env!("CARGO_BIN_EXE_robigo-rt")), args, env, &[], stdin)
}

/// [`boot`] with the robigo-rt at `bin` (its R_HOME the install layout's:
/// `lib/robigo` beside its `bin`).
fn boot_bin(
    bin: &std::path::Path,
    args: &[&std::ffi::OsStr],
    env: &[(&str, &std::ffi::OsStr)],
) -> (String, String, Option<i32>) {
    boot_bin_reading(bin, args, env, &[], None)
}

/// [`boot_bin`] with `stdin` as the console's input (None: no stdin), and
/// the variables `remove` removed last.
fn boot_bin_reading(
    bin: &std::path::Path,
    args: &[&std::ffi::OsStr],
    env: &[(&str, &std::ffi::OsStr)],
    remove: &[&str],
    stdin: Option<&[u8]>,
) -> (String, String, Option<i32>) {
    // one directory per run (the tests run in parallel)
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!("robigo-boot-db-home-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .current_dir(&home)
        // a private cwd and HOME: no caller's .Renviron or .Rprofile
        .env("HOME", &home)
        .env("ROBIGO_RT_BOOT", "rdb")
        .env("R_DEFAULT_PACKAGES", "NULL")
        .env("LC_ALL", "C.UTF-8")
        .env_remove("R_HOME")
        .env_remove("ROBIGO_RT_CHECKOUT")
        // the profile's own controls (Common.R's .Last.sys under R_BATCH,
        // Rprofile.unix's R_TESTS file): not the boot under test
        .env_remove("R_BATCH")
        .env_remove("R_TESTS")
        .env_remove("R_ENVIRON")
        .env_remove("R_PROFILE")
        .env_remove("R_ENVIRON_USER")
        .env_remove("R_PROFILE_USER")
        // what the system Renviron and the tests' files read: the system
        // Renviron's own values, not the caller's
        .env_remove("R_BROWSER")
        .env_remove("R_PAPERSIZE")
        .env_remove("ROBIGO_T_UNSET")
        // R_init_jit_enabled's and R_CheckJIT's
        .env_remove("R_ENABLE_JIT")
        .env_remove("R_JIT_STRATEGY")
        .env_remove("R_MIN_JIT_SCORE")
        .env_remove("R_DISABLE_BYTECODE")
        .env_remove("_R_COMPILE_PKGS_")
        // rt-ci runs the tests under its observation modes (scripts/rt-ci.py
        // inherits the environment): the boot under them is not this test's
        .env_remove("ROBIGO_GCTORTURE")
        .env_remove("ROBIGO_GC_STATS")
        .env_remove("ROBIGO_GC_MOVE")
        .env_remove("ROBIGO_GC_PIN_REPORT")
        .env_remove("ROBIGO_FRAME_QUARANTINE")
        .env_remove("ROBIGO_FAIL_ALLOC_AT")
        .env_remove("ROBIGO_NO_INPLACE")
        .env_remove("ROBIGO_MATERIALIZE")
        .env_remove("ROBIGO_BOOT_VERBOSE")
        // the translation cache's controls (C1): the tree's cache, on
        .env_remove("ROBIGO_TRANSLATION_CACHE")
        .env_remove("ROBIGO_TRANSLATION_CACHE_STATS")
        .env_remove("ROBIGO_TRANSLATION_CACHE_WRITE")
        // the test's own, last (over the removals above)
        .envs(env.iter().copied());
    for v in remove {
        cmd.env_remove(v);
    }
    let mut o = cmd
        .stdin(if stdin.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("running robigo-rt: {e}"));
    if let Some(bytes) = stdin {
        // (all of it, then the end: the console reads it as it goes; a run
        // that halts first, as a script at a start-up error does, may close
        // the pipe before the write lands: cli.rs's `run` allows that too)
        let mut w = o.stdin.take().unwrap();
        match std::io::Write::write_all(&mut w, bytes) {
            Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => panic!("writing robigo-rt's stdin: {e}"),
            _ => {}
        }
    }
    let o = o.wait_with_output().unwrap_or_else(|e| panic!("running robigo-rt: {e}"));
    let _ = std::fs::remove_dir_all(&home);
    (String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned(), o.status.code())
}

#[test]
fn base_boots_from_its_database() {
    if !tree_made() {
        return;
    }
    let expr = "f <- get('paste', baseenv()); \
                cat(typeof(.Internal(bodyCode(f))), exists('.C_R_addTaskCallback'), exists('.F_dqrdc2'), \
                    length(ls(.BaseNamespaceEnv[['.__S3MethodsTable__.']], all.names = TRUE)) == nrow(.S3_methods_table), \
                    environmentIsLocked(.ArgsEnv), .Device, length(ls(baseenv(), all.names = TRUE)), \
                    exists('.Library.site', envir = globalenv(), inherits = FALSE), getOption('browser'), '\\n')";
    let (out, err, status) = boot(&["--vanilla".as_ref(), "-e".as_ref(), expr.as_ref()], &[]);
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        ("bytecode TRUE TRUE TRUE TRUE null device 1407 FALSE xdg-open \n", PROFILE_STOPS, Some(0))
    );
}

/// The start-up files after the system profile, as GNU R 4.6.2 reads them
/// (Rscript without --vanilla: the user's Renviron and profile read): a
/// user Renviron's terms and quotes, its invalid line's message (on stdout
/// this early), the user profile in the global environment, `.First`, and
/// `.Library.site` gone from it. Recorded on the oracle with the same files.
#[test]
fn the_start_up_files_are_read_as_gnu_r_reads_them() {
    if !tree_made() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("robigo-boot-db-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let renviron = dir.join("renviron");
    std::fs::write(
        &renviron,
        "# comment\nROBIGO_T_A=plain\nROBIGO_T_B=\"quoted value\"\nROBIGO_T_C=${ROBIGO_T_UNSET-'default'}/x\n\
         ROBIGO_T_D=${ROBIGO_T_A:-no}\nnot a line\n",
    )
    .unwrap();
    let profile = dir.join("profile");
    std::fs::write(&profile, "cat(\"user profile\\n\")\n.First <- function() cat(\".First ran\\n\")\n").unwrap();
    let script = dir.join("script.R");
    std::fs::write(
        &script,
        "cat(Sys.getenv(c(\"ROBIGO_T_A\", \"ROBIGO_T_B\", \"ROBIGO_T_C\", \"ROBIGO_T_D\")), sep = \"|\"); cat(\"\\n\")\n\
         cat(exists(\".Library.site\", envir = globalenv(), inherits = FALSE), getOption(\"browser\"), Sys.getenv(\"R_PAPERSIZE\"), \"\\n\")\n",
    )
    .unwrap();
    let (out, err, status) = boot(
        &[script.as_os_str()],
        &[("R_ENVIRON_USER", renviron.as_os_str()), ("R_PROFILE_USER", profile.as_os_str())],
    );
    let gnur = format!(
        "\n   File {} contains invalid line(s)\n      not a line\n   They were ignored\n\n\
         user profile\n.First ran\nplain|quoted value|default/x|plain\nFALSE xdg-open letter \n",
        renviron.display()
    );
    // .First's error halts R (check_session_exit), .Last not run
    let profile2 = dir.join("profile2");
    std::fs::write(&profile2, ".First <- function() stop(\"boom\")\n").unwrap();
    // (an empty R_ENVIRON_USER reads no user Renviron: not the caller's)
    let (out2, err2, status2) =
        boot(&[script.as_os_str()], &[("R_PROFILE_USER", profile2.as_os_str()), ("R_ENVIRON_USER", "".as_ref())]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!((out.as_str(), err.as_str(), status), (gnur.as_str(), PROFILE_STOPS, Some(0)));
    let halted = format!("{PROFILE_STOPS}Error in .First() : boom\nExecution halted\n");
    assert_eq!((out2.as_str(), err2.as_str(), status2), ("", halted.as_str(), Some(1)));
}

/// A user profile's errors as GNU R 4.6.2 handles them (recorded on the
/// oracle): `R_LoadProfile`'s SETJMP, not R_ReplFile's, takes them, so the
/// statements before a syntax error run, the error (with the failing
/// statement's context alone) halts the script, and with
/// `options(catch.script.errors = TRUE)` the rest of the profile is not
/// read and the script runs.
#[test]
fn a_user_profile_s_errors_end_it_as_in_gnu_r() {
    if !tree_made() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("robigo-boot-db-profile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("s.R");
    std::fs::write(&script, "cat(\"script ran\\n\")\n").unwrap();
    let run = |name: &str, text: &str| {
        let f = dir.join(name);
        std::fs::write(&f, text).unwrap();
        boot(&[script.as_os_str()], &[("R_PROFILE_USER", f.as_os_str()), ("R_ENVIRON_USER", "".as_ref())])
    };
    let parse = run("parse", "cat(\"before\\n\")\n)\ncat(\"after\\n\")\n");
    let error = run("error", "cat(\"one\\n\")\nstop(\"boom\")\ncat(\"continued\\n\")\n");
    let caught =
        run("caught", "options(catch.script.errors = TRUE)\ncat(\"one\\n\")\nstop(\"boom\")\ncat(\"continued\\n\")\n");
    // a lexical error is raiseLexError's condition; the lexer's warnings go
    // with their statement (an error under warn = 2, else printed after it)
    let lex = run("lex", "cat(\"a\\n\")\nx <- \"\\q\"\ncat(\"b\\n\")\n");
    let warn2 = run("warn2", "options(warn = 2)\nx <- 1.1L\ncat(\"continued\\n\")\n");
    let warn = run("warn", "x <- 1.1L\ncat(\"continued\\n\")\n");
    // a code-generation error after the lookahead was lexed: its warning
    // fired first (an error under warn = 2, else "In addition")
    let formal2 = run("formal2", "options(warn = 2)\nf <- function(x, x = 1.1L) {}\n");
    let formal = run("formal", "f <- function(x, x = 1.1L) {}\n");
    // a grammar error's frontier is the token it rejects: the literal after
    // it is not lexed, so its warning does not fire
    let reject = run("reject", "options(warn = 2)\n) 1.1L\n");
    // an R error raised with no lookahead read (a zero-length name): the
    // literal after it is not lexed either
    let zero = run("zero", "options(warn = 2)\n\"\"() 1.1L\n");
    // an empty tag's error comes after the lookahead was read: the
    // literal's warning first
    let tag = run("tag", "options(warn = 2)\nf(\"\" = 1 1.1L)\n");
    let _ = std::fs::remove_dir_all(&dir);
    let lex_err = format!(
        "{PROFILE_STOPS}Error: '\\q' is an unrecognized escape in character string (<input>:2:8)\nExecution halted\n"
    );
    assert_eq!((lex.0.as_str(), lex.1.as_str(), lex.2), ("a\n", lex_err.as_str(), Some(1)));
    let warn2_err = format!(
        "{PROFILE_STOPS}Error: (converted from warning) integer literal 1.1L contains decimal; using numeric value\n\
         Execution halted\n"
    );
    assert_eq!((warn2.0.as_str(), warn2.1.as_str(), warn2.2), ("", warn2_err.as_str(), Some(1)));
    let warn_err =
        format!("{PROFILE_STOPS}Warning message:\ninteger literal 1.1L contains decimal; using numeric value \n");
    assert_eq!((warn.0.as_str(), warn.1.as_str(), warn.2), ("continued\nscript ran\n", warn_err.as_str(), Some(0)));
    assert_eq!((formal2.0.as_str(), formal2.1.as_str(), formal2.2), ("", warn2_err.as_str(), Some(1)));
    let formal_err = format!(
        "{PROFILE_STOPS}Error: repeated formal argument 'x' (<input>:1:18)\nIn addition: Warning message:\n\
         integer literal 1.1L contains decimal; using numeric value \nExecution halted\n"
    );
    assert_eq!((formal.0.as_str(), formal.1.as_str(), formal.2), ("", formal_err.as_str(), Some(1)));
    let reject_err = format!("{PROFILE_STOPS}Error: 2:1: unexpected ')'\n2: )\n    ^\nExecution halted\n");
    assert_eq!((reject.0.as_str(), reject.1.as_str(), reject.2), ("", reject_err.as_str(), Some(1)));
    let zero_err = format!("{PROFILE_STOPS}Error: attempt to use zero-length variable name\nExecution halted\n");
    assert_eq!((zero.0.as_str(), zero.1.as_str(), zero.2), ("", zero_err.as_str(), Some(1)));
    assert_eq!((tag.0.as_str(), tag.1.as_str(), tag.2), ("", warn2_err.as_str(), Some(1)));
    let parse_err = format!("{PROFILE_STOPS}Error: 2:1: unexpected ')'\n2: )\n    ^\nExecution halted\n");
    assert_eq!((parse.0.as_str(), parse.1.as_str(), parse.2), ("before\n", parse_err.as_str(), Some(1)));
    let error_err = format!("{PROFILE_STOPS}Error: boom\nExecution halted\n");
    assert_eq!((error.0.as_str(), error.1.as_str(), error.2), ("one\n", error_err.as_str(), Some(1)));
    let caught_err = format!("{PROFILE_STOPS}Error: boom\n");
    assert_eq!((caught.0.as_str(), caught.1.as_str(), caught.2), ("one\nscript ran\n", caught_err.as_str(), Some(0)));
}

/// The tree declares the default packages it has (scripts/rt-home-db.sh's
/// line in etc/Renviron, R_DEFAULT_PACKAGES): none today, so a start with
/// no R_DEFAULT_PACKAGES of its own attaches none and says nothing, and
/// `library()` of one still fails as GNU R's does in a tree without it
/// (find.package's short cut for the standard packages gives
/// `.Library/stats`, which holds no installed package: recorded on 4.6.2
/// with stats removed from the tree, in a throwaway container).
#[test]
fn the_default_start_attaches_the_packages_the_tree_has() {
    if !tree_made() {
        return;
    }
    let expr = "cat(length(getOption('defaultPackages')), search(), '\\n'); \
                tryCatch(library(stats), error = function(e) cat(conditionMessage(e), '\\n'))";
    let (out, err, status) =
        boot_without(&["--vanilla".as_ref(), "-e".as_ref(), expr.as_ref()], &[], &["R_DEFAULT_PACKAGES"]);
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        (
            "0 .GlobalEnv Autoloads package:base \n\u{2018}stats\u{2019} is not a valid installed package \n",
            "",
            Some(0)
        )
    );
}

/// A user's own R_DEFAULT_PACKAGES is kept over the tree's line: a package
/// the tree lacks gives GNU R's warning at the end of the start (recorded
/// on 4.6.2 with a package GNU R lacks, under C.UTF-8).
#[test]
fn a_user_s_default_packages_warn_as_in_gnu_r() {
    if !tree_made() {
        return;
    }
    let (out, err, status) = boot(
        &["--vanilla".as_ref(), "-e".as_ref(), "cat('ran\\n')".as_ref()],
        &[("R_DEFAULT_PACKAGES", "stats".as_ref())],
    );
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        (
            "ran\n",
            "During startup - Warning message:\npackage \u{2018}stats\u{2019} in options(\"defaultPackages\") was not found \n",
            Some(0)
        )
    );
}

/// An unknown `ROBIGO_RT_BOOT` (a typo) is an error, not the default boot;
/// empty is unset.
#[test]
fn an_unknown_boot_is_an_error() {
    let o = Command::new(env!("CARGO_BIN_EXE_robigo-rt"))
        .args(["--vanilla", "-e", "1"])
        .env("ROBIGO_RT_BOOT", "rbd")
        .env_remove("R_HOME")
        .output()
        .unwrap();
    assert_eq!(
        (String::from_utf8_lossy(&o.stdout).as_ref(), String::from_utf8_lossy(&o.stderr).as_ref(), o.status.code()),
        (
            "",
            "robigo-rt: ROBIGO_RT_BOOT is 'rbd': it is 'rdb' (the default, also when unset), 'stage0' or 'm1' (M1's \
             loader)\n",
            Some(2)
        )
    );
}

#[test]
fn the_jit_starts_as_setup_rmainloop_ends() {
    if !tree_made() {
        return;
    }
    // GNU R 4.6.2's (R_DEFAULT_PACKAGES=NULL R --vanilla --no-echo -e ...):
    // the compiler loaded at the end of the start when the JIT is on, its
    // closures byte code from compiler.rdb; the JIT's level, then
    // _R_COMPILE_PKGS_ and R_DISABLE_BYTECODE read there
    // (a closure the JIT compiled is not yet byte code to bodyCode: C1's
    // V4 step 3, #931; then add GNU R's `f <- function(x) x + 1; for (i in
    // 1:3) f(i)` giving "bytecode", "language" with R_ENABLE_JIT=0 or
    // R_DISABLE_BYTECODE=1, and R_JIT_STRATEGY's default read at base's
    // first closure call, the JIT still off: with R_ENABLE_JIT=1 `g <-
    // function() 1` is byte code after its second call, TOP_SMALL_MAYBE)
    let state = "cat(loadedNamespaces(), typeof(.Internal(bodyCode(compiler::cmpfun))), \
                 compiler::enableJIT(-1), compiler::compilePKGS(FALSE), '\\n')";
    let cases: &[(&[(&str, &str)], &str)] = &[
        (&[], "compiler base bytecode 3 FALSE \n"),
        (&[("R_ENABLE_JIT", "0")], "base bytecode 0 FALSE \n"),
        (&[("R_ENABLE_JIT", "1")], "compiler base bytecode 1 FALSE \n"),
        (&[("_R_COMPILE_PKGS_", "1")], "compiler base bytecode 3 TRUE \n"),
    ];
    for (env, want) in cases {
        let env: Vec<(&str, &std::ffi::OsStr)> = env.iter().map(|(k, v)| (*k, v.as_ref())).collect();
        let (out, err, status) =
            boot(&["--vanilla".as_ref(), "--no-echo".as_ref(), "-e".as_ref(), state.as_ref()], &env);
        assert_eq!((out.as_str(), err.as_str(), status), (*want, "", Some(0)), "{env:?}");
    }
    // the JIT is off while the profiles run
    let dir = std::env::temp_dir().join(format!("robigo-boot-db-jit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let prof = dir.join("profile.R");
    std::fs::write(&prof, "cat('prof', compiler::enableJIT(-1), '\\n')\n").unwrap();
    let (out, _, status) = boot(
        &[
            "--no-save".as_ref(),
            "--no-restore".as_ref(),
            "--no-echo".as_ref(),
            "-e".as_ref(),
            "cat(compiler::enableJIT(-1), '\\n')".as_ref(),
        ],
        &[("R_PROFILE_USER", prof.as_os_str())],
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!((out.as_str(), status), ("prof 0 \n3 \n", Some(0)));
}

#[test]
fn a_named_profile_that_does_not_open_is_no_profile() {
    if !tree_made() {
        return;
    }
    // R_OpenSiteFile and R_OpenInitFile return R_fopen's NULL, which
    // R_LoadProfile skips (startup.c:115, sys-unix.c:77, main.c:723); GNU R
    // 4.6.2 runs on, as here
    let missing = std::ffi::OsStr::new("/nonexistent/robigo-profile.R");
    let (out, err, status) = boot(
        &["--no-save".as_ref(), "--no-restore".as_ref(), "--no-echo".as_ref(), "-e".as_ref(), "cat('ran\\n')".as_ref()],
        &[("R_PROFILE", missing), ("R_PROFILE_USER", missing)],
    );
    assert_eq!((out.as_str(), err.as_str(), status), ("ran\n", PROFILE_STOPS, Some(0)));
}

/// A copy of the tree in a scratch checkout named for `what`, with this
/// binary's translation cache for it, as scripts/rt-home-db.sh writes one
/// (the binary under test need not be the one that built the tree, whose
/// cache would be another build's).
fn cached_tree(what: &str) -> Scratch {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tmp = std::env::temp_dir().join(format!("robigo-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    // (removed when it drops: an assert's unwind below or in the test too)
    let tmp = Scratch(tmp);
    std::fs::create_dir_all(tmp.join(".robigo")).unwrap();
    let st =
        Command::new("cp").arg("-a").arg(root.join(".robigo/rt-home-db")).arg(tmp.join(".robigo/rt-home-db")).status();
    assert!(st.unwrap().success(), "copying the tree");
    std::fs::create_dir_all(tmp.join(".robigo/rt-home-db-cache")).unwrap();
    write_cache(&tmp);
    tmp
}

/// A scratch tree (a full copy of the database tree and its cache) that
/// removes itself when dropped, on every exit path of a test.
struct Scratch(std::path::PathBuf);

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// This binary's writer run on the tree copy `tmp` (its `-cache` directory
/// there).
fn write_cache(tmp: &std::path::Path) {
    let mine = tmp.join(".robigo/rt-home-db-cache");
    let force = "for (p in c('base', 'compiler')) { ns <- asNamespace(p); for (n in ls(ns, all.names = TRUE)) \
                 tryCatch(get(n, envir = ns), error = function(e) NULL) }";
    let (_, err, st) = boot(
        &[std::ffi::OsStr::new("--vanilla"), std::ffi::OsStr::new("-e"), std::ffi::OsStr::new(force)],
        &[("ROBIGO_RT_CHECKOUT", tmp.as_os_str()), ("ROBIGO_TRANSLATION_CACHE_WRITE", std::ffi::OsStr::new("1"))],
    );
    assert_eq!(st, Some(0), "the writer: {err}");
    assert!(mine.join("base.rtc").is_file() && mine.join("compiler.rtc").is_file(), "the writer wrote no files: {err}");
}

/// The writer replaces a cache file by rename, never writing an existing
/// one in place: a process that maps the old file (the reader does) keeps
/// its bytes, which a write or truncation in place would change under it
/// (or end it with SIGBUS).
#[test]
fn the_writer_replaces_a_cache_file_by_rename() {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    if !tree_made() {
        return;
    }
    let tmp = cached_tree("tcache-rename");
    let path = tmp.join(".robigo/rt-home-db-cache/base.rtc");
    let mut old = std::fs::File::open(&path).unwrap();
    let ino = old.metadata().unwrap().ino();
    let before = std::fs::read(&path).unwrap();
    write_cache(&tmp);
    let after = std::fs::metadata(&path).unwrap();
    let mut kept = Vec::new();
    old.read_to_end(&mut kept).unwrap();
    let leftovers: Vec<_> = std::fs::read_dir(tmp.join(".robigo/rt-home-db-cache"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| !n.ends_with(".rtc"))
        .collect();
    assert_ne!(after.ino(), ino, "base.rtc was written in place, not replaced");
    assert_eq!(kept, before, "the old file's bytes changed under an open handle");
    assert!(leftovers.is_empty(), "the writer left {leftovers:?}");
}

/// The writer and the verify pass in one run would verify nothing (the
/// write pass translates every body first): the combination is refused,
/// with a message, not passed.
#[test]
fn writing_and_verifying_the_cache_in_one_run_is_refused() {
    if !tree_made() {
        return;
    }
    let tmp = cached_tree("tcache-write-verify");
    let (_, err, st) = boot(
        &[std::ffi::OsStr::new("--vanilla"), std::ffi::OsStr::new("-e"), std::ffi::OsStr::new("1")],
        &[
            ("ROBIGO_RT_CHECKOUT", tmp.as_os_str()),
            ("ROBIGO_TRANSLATION_CACHE_WRITE", std::ffi::OsStr::new("1")),
            ("ROBIGO_TRANSLATION_CACHE", std::ffi::OsStr::new("verify")),
        ],
    );
    assert_ne!(st, Some(0), "{err}");
    assert!(err.contains("together verify nothing"), "{err}");
}

/// A verification whose fresh translation stops with a retryable `Need`
/// compares nothing and leaves nothing for the next one: the attempt runs
/// again and verifies then. Under ROBIGO_RETRY_TORTURE (the retry-torture
/// build: rt-ci's feature-builds shard runs this test there) every
/// attempt's first allocation is refused, so every fresh translation of a
/// verification stops once inside the verification: none may count as
/// differing. In a build without the feature the binary refuses the knob,
/// and the test has nothing to run.
#[test]
fn a_verification_stopped_for_a_retry_compares_nothing() {
    if !tree_made() {
        return;
    }
    let (_, err, st) = boot(
        &[std::ffi::OsStr::new("--vanilla"), std::ffi::OsStr::new("-e"), std::ffi::OsStr::new("1")],
        &[("ROBIGO_RETRY_TORTURE", std::ffi::OsStr::new("1"))],
    );
    // the knob's refusal (a build without the feature) is the only skip;
    // any other failure of the probe is loud, and rt-ci's step, which runs
    // the feature build, sets ROBIGO_REQUIRE_RETRY_TORTURE so that a
    // refusal fails there too
    let refused = st != Some(0) && err.contains("needs robigo-rtcore built with its retry-torture feature");
    assert!(refused || st == Some(0), "the probe failed: {st:?} {err}");
    if refused {
        assert!(
            std::env::var_os("ROBIGO_REQUIRE_RETRY_TORTURE").is_none(),
            "ROBIGO_REQUIRE_RETRY_TORTURE is set, but this build has no retry-torture: {err}"
        );
        return;
    }
    let tmp = cached_tree("tcache-verify-retry");
    let report = tmp.join("crosscheck.txt");
    let (out, err, st) = boot(
        &[
            std::ffi::OsStr::new("--vanilla"),
            std::ffi::OsStr::new("-e"),
            std::ffi::OsStr::new("cat(sum(1:10), \"\\n\")"),
        ],
        &[
            ("ROBIGO_RT_CHECKOUT", tmp.as_os_str()),
            // every hit of the start verified (the crosscheck compares,
            // never trusts), without ROBIGO_TRANSLATION_CACHE=verify's pass
            // over the whole cache, which under the knob takes half an hour:
            // the JIT off, a start's hits are about 140
            ("ROBIGO_BC_CROSSCHECK", std::ffi::OsStr::new("1")),
            ("ROBIGO_BC_CROSSCHECK_REPORT", report.as_os_str()),
            ("R_ENABLE_JIT", std::ffi::OsStr::new("0")),
            ("ROBIGO_TRANSLATION_CACHE_STATS", std::ffi::OsStr::new("1")),
            ("ROBIGO_RETRY_TORTURE", std::ffi::OsStr::new("1")),
        ],
    );
    assert_eq!((out.as_str(), st), ("55 \n", Some(0)), "{err}");
    let line = err.lines().find(|l| l.starts_with("translation cache: hit ")).unwrap_or_else(|| panic!("{err}"));
    // ("name N" fields, comma-separated: "verified" is not "unverified")
    let count = |what: &str| -> usize {
        let field = line["translation cache: ".len()..]
            .split(", ")
            .find_map(|f| f.strip_prefix(what).and_then(|n| n.strip_prefix(' ')))
            .unwrap_or_else(|| panic!("no '{what}' in {line}"));
        field.trim().parse().unwrap()
    };
    assert!(count("verify retried") > 0, "no verification stopped for a retry: {line}");
    assert!(count("verified") > 0, "nothing verified: {line}");
    assert_eq!(count("differing"), 0, "{line}");
}

/// The translation cache beside the tree (C1, crates/robigo-rt/src/tcache.rs):
/// a copy of the tree gets its cache from this binary's writer (the binary
/// under test need not be the one that built the tree, whose cache would be
/// another build's), and a start from it uses it; with the cache absent,
/// stale (another build's: its build ID changed) or unreadable, a start is
/// as with none: the same output, no message, every lookup a miss.
#[test]
fn a_start_falls_back_silently_without_a_valid_translation_cache() {
    use std::os::unix::fs::PermissionsExt;
    if !tree_made() {
        return;
    }
    let tmp = cached_tree("tcache");
    let mine = tmp.join(".robigo/rt-home-db-cache");
    let cp = |from: &std::path::Path, to: &std::path::Path| {
        let st = Command::new("cp").arg("-a").arg(from).arg(to).status().unwrap();
        assert!(st.success(), "cp -a {} {}", from.display(), to.display());
    };
    let cache = tmp.join("cache-written");
    cp(&mine, &cache);
    let run = || {
        boot(
            &[
                std::ffi::OsStr::new("--vanilla"),
                std::ffi::OsStr::new("-e"),
                std::ffi::OsStr::new("cat(sum(1:10), \"\\n\")"),
            ],
            &[("ROBIGO_RT_CHECKOUT", tmp.as_os_str()), ("ROBIGO_TRANSLATION_CACHE_STATS", std::ffi::OsStr::new("1"))],
        )
    };
    let hits = |err: &str| -> usize {
        let line = err.lines().last().unwrap_or("");
        assert!(line.starts_with("translation cache: hit "), "{err}");
        assert_eq!(err.lines().count(), 1, "a start said more than its counts: {err}");
        line["translation cache: hit ".len()..].split(',').next().unwrap().parse().unwrap()
    };
    // present
    let (out, err, st) = run();
    assert_eq!((out.as_str(), st), ("55 \n", Some(0)), "{err}");
    assert!(hits(&err) > 0, "the cache was not used: {err}");
    // stale: another build's (its build ID's first byte changed)
    for f in ["base.rtc", "compiler.rtc"] {
        let p = mine.join(f);
        let mut b = std::fs::read(&p).unwrap();
        b[robigo_rt::TRANSLATION_CACHE_BUILD_ID_AT] ^= 0xff;
        std::fs::write(&p, b).unwrap();
    }
    let (out, err, st) = run();
    assert_eq!((out.as_str(), st, hits(&err)), ("55 \n", Some(0), 0), "stale: {err}");
    // unreadable, everywhere: a directory where each file is (its read
    // fails with EISDIR, to root as to anyone; CI's containers run as root)
    std::fs::remove_dir_all(&mine).unwrap();
    std::fs::create_dir_all(&mine).unwrap();
    for f in ["base.rtc", "compiler.rtc"] {
        std::fs::create_dir(mine.join(f)).unwrap();
    }
    let (out, err, st) = run();
    assert_eq!((out.as_str(), st, hits(&err)), ("55 \n", Some(0), 0), "unreadable (a directory): {err}");
    // and a file of mode 000, where that is unreadable (not to root)
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        std::fs::remove_dir_all(&mine).unwrap();
        cp(&cache, &mine);
        for f in ["base.rtc", "compiler.rtc"] {
            std::fs::set_permissions(mine.join(f), std::fs::Permissions::from_mode(0o000)).unwrap();
        }
        let (out, err, st) = run();
        assert_eq!((out.as_str(), st, hits(&err)), ("55 \n", Some(0), 0), "unreadable (mode 000): {err}");
        for f in ["base.rtc", "compiler.rtc"] {
            std::fs::set_permissions(mine.join(f), std::fs::Permissions::from_mode(0o644)).unwrap();
        }
    }
    // absent
    std::fs::remove_dir_all(&mine).unwrap();
    let (out, err, st) = run();
    assert_eq!((out.as_str(), st, hits(&err)), ("55 \n", Some(0), 0), "absent: {err}");
}

/// R_Interactive, set before base and the profiles (Rf_initialize_R), makes
/// every start-up step's error go on, as GNU R 4.6.2 does (recorded on the
/// oracle, `R --no-readline --interactive --no-save --quiet < input`):
/// setup_Rmainloop runs each step under its own `SETJMP(R_Toplevel.cjmpbuf)`
/// whose `check_session_exit` halts only `if (! R_Interactive)` (main.c:331).
/// The site profile's and the user profile's rest is not read, `.First`'s
/// error is reported, and the console reads its input. The same files halt
/// a script at the site profile's error.
#[test]
fn start_up_errors_go_on_to_the_interactive_prompt() {
    if !tree_made() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("robigo-boot-db-interactive-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let site = dir.join("site.R");
    std::fs::write(&site, "cat(\"site one\\n\")\nstop(\"site boom\")\ncat(\"site rest\\n\")\n").unwrap();
    let user = dir.join("user.R");
    std::fs::write(
        &user,
        "cat(\"user one\\n\")\n.First <- function() { cat(\"first\\n\"); stop(\"first boom\") }\n\
         stop(\"user boom\")\ncat(\"user rest\\n\")\n",
    )
    .unwrap();
    let env = [("R_PROFILE", site.as_os_str()), ("R_PROFILE_USER", user.as_os_str()), ("R_ENVIRON_USER", "".as_ref())];
    let input = b"cat(\"console\\n\")\n1 + 1\n";
    let interactive = boot_reading(
        &["--no-readline".as_ref(), "--interactive".as_ref(), "--no-save".as_ref(), "--quiet".as_ref()],
        &env,
        Some(input),
    );
    let script = boot_reading(&["--no-save".as_ref(), "--quiet".as_ref()], &env, Some(input));
    let _ = std::fs::remove_dir_all(&dir);
    // INTERIM(keep.source): the system profile's options(keep.source =
    // interactive()) is refused (no source references yet); GNU R's stderr
    // does not have this error, and its keep.source is TRUE
    let want_err =
        format!("{INTERACTIVE_PROFILE_STOPS}Error: site boom\nError: user boom\nError in .First() : first boom\n");
    assert_eq!(
        (interactive.0.as_str(), interactive.1.as_str(), interactive.2),
        ("site one\nuser one\nfirst\n> console\n> [1] 2\n> \n", want_err.as_str(), Some(0))
    );
    let halted = format!("{PROFILE_STOPS}Error: site boom\nExecution halted\n");
    assert_eq!((script.0.as_str(), script.1.as_str(), script.2), ("site one\n", halted.as_str(), Some(1)));
}

/// What the system profile's interactive start stops at in robigo-rt and
/// GNU R does not (INTERIM(keep.source)).
const INTERACTIVE_PROFILE_STOPS: &str =
    "Error in options(...) : \n  robigo: options(keep.source = TRUE) (source references) is not implemented yet\n";

#[test]
fn a_broken_compiler_database_ends_the_start_as_in_gnu_r() {
    if !tree_made() {
        return;
    }
    // R_init_jit_enabled's loadCompilerNamespace fails: check_session_exit
    // halts R, or with options(error) lets it go on to R_Suicide("unable to
    // initialize the JIT") (main.c:1205-1214, eval.c:1415); GNU R 4.6.2's
    // output with its compiler's .rdb and .rdx removed (an emptied .rdb is
    // not a case: GNU R's readRawFromFile then copies the record from past
    // its zero-length cache, serialize.c:3146, and inflates what it read)
    let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.robigo/rt-home-db");
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    // (the install layout robigo-rt finds its R_HOME by: DIR/bin/robigo-rt,
    // DIR/lib/robigo, a copy of the tree; an R_HOME in the environment is
    // ignored, as GNU R's is)
    let copy = |what: &str| {
        let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("robigo-boot-db-{what}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_robigo-rt"), dir.join("bin/robigo-rt")).unwrap();
        let ok = Command::new("cp").arg("-a").arg(&tree).arg(dir.join("lib/robigo")).status().unwrap().success();
        assert!(ok, "copying {}", tree.display());
        dir
    };
    let ran: &[&std::ffi::OsStr] =
        &["--vanilla".as_ref(), "--no-echo".as_ref(), "-e".as_ref(), "cat('ran\\n')".as_ref()];
    // (removed when the test ends, an assertion's panic included)
    struct Gone(std::path::PathBuf);
    impl Drop for Gone {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = copy("removed");
    let _gone = Gone(dir.clone());
    let bin = dir.join("bin/robigo-rt");
    let home = std::fs::canonicalize(dir.join("lib/robigo")).unwrap();
    std::fs::remove_file(home.join("library/compiler/R/compiler.rdb")).unwrap();
    std::fs::remove_file(home.join("library/compiler/R/compiler.rdx")).unwrap();
    let errors = format!(
        "Error in gzfile(file, \"rb\") : cannot open the connection\nIn addition: Warning message:\n\
         In gzfile(file, \"rb\") :\n  cannot open compressed file '{}/library/compiler/R/compiler.rdx', probable reason 'No such file or directory'\n\
         Error: unable to load R code in package ‘compiler’\n",
        home.display()
    );
    // check_session_exit halts R
    let (out, err, status) = boot_bin(&bin, ran, &[]);
    assert_eq!((out.as_str(), err, status), ("", format!("{errors}Execution halted\n"), Some(1)));
    // with options(error) it lets R go on, to R_Suicide
    let prof = dir.join("prof.R");
    std::fs::write(&prof, "options(error = function() NULL)\n").unwrap();
    let (out, err, status) = boot_bin(
        &bin,
        &["--no-save".as_ref(), "--no-restore".as_ref(), "--no-echo".as_ref(), "-e".as_ref(), "cat('ran\\n')".as_ref()],
        &[("R_PROFILE_USER", prof.as_os_str())],
    );
    assert_eq!(
        (out.as_str(), err, status),
        ("", format!("{errors}Fatal error: unable to initialize the JIT\n\n"), Some(2))
    );
    // the JIT off: no compiler at the start
    let (out, err, status) = boot_bin(&bin, ran, &[("R_ENABLE_JIT", "0".as_ref())]);
    assert_eq!((out.as_str(), err.as_str(), status), ("ran\n", PROFILE_STOPS, Some(0)));
}

/// The boot heap's freeze in steady state (step 2, the foreman's clause):
/// once a workload has run once, running it again collects without a
/// frozen-reachability pass. Each shape runs a warm-up and then 0 or
/// `ROUNDS` more rounds, a collection after each; the passes
/// (`ROBIGO_GC_STATS`) must be the same, so a write that notes in a loop
/// fails here and names its shape. Dispatch cycles 400 generics through S3's method memo (256
/// entries), which replaced its entries' globals at every call: before
/// globals stopped being freeze roots, a pass at every collection. Run with
/// the freeze's check at every collection, so that a missed note (frozen
/// garbage no pass removes) fails here too.
#[test]
fn no_frozen_reachability_pass_in_steady_state() {
    if !tree_made() {
        return;
    }
    // (name, setup, body, iterations per round, gctorture: a collection at
    // every allocation, as rt-ci's E3 steps run: deep frames there were the
    // case the foreman named, tests/r/lang/tailcall-frames.R. Turned on by
    // the script after its setup, `gctorture(TRUE)`, so the shape's own
    // work runs under it and the boot does not: the boot under gctorture
    // is E3's and the crates' gctorture step's, and it was most of this
    // test's time with debug assertions on)
    const SHAPES: &[(&str, &str, &str, u32, bool)] = &[
        (
            "dispatch",
            "for (k in 1:400) { assign(paste0(\"gen\", k), eval(bquote(function(x) UseMethod(.(paste0(\"gen\", k)))))); assign(paste0(\"gen\", k, \".default\"), function(x) 1L) }\ngens <- mget(paste0(\"gen\", 1:400))\nobj <- structure(list(), class = \"thing\")",
            "s <- 0L; for (f in gens) s <- s + f(obj)",
            3,
            false,
        ),
        (
            "conditions",
            "safe <- function(x) tryCatch(if (x > 0) x else stop(\"neg\"), error = function(e) 0)\nw <- function(x) withCallingHandlers(if (x > 0) warning(\"w\") else x, warning = function(w) invokeRestart(\"muffleWarning\"))",
            "s <- safe(i %% 2L - 0.5); t <- w(i %% 2L)",
            2000,
            false,
        ),
        (
            "apply",
            "x <- 1:50; f <- function(v) v * 2",
            "a <- lapply(x, f); b <- vapply(x, f, numeric(1)); c <- mapply(function(p, q) p + q, x, x); d <- Map(f, x)",
            200,
            false,
        ),
        (
            "environments",
            "mk <- function(i) { e <- new.env(); assign('v', i, envir = e); local({ z <- v + 1 }, envir = e); e }",
            "e <- mk(i); g <- get('v', envir = e); h <- function() g; k <- h()",
            3000,
            false,
        ),
        (
            "jit",
            "",
            "f <- eval(parse(text = sprintf('function(x) { y <- x + %d; for (j in 1:3) y <- y + j; y }', i))); s <- f(1) + f(2) + f(3)",
            300,
            false,
        ),
        ("deep frames under gctorture", "f <- function(n) if (n == 0) 0 else n + f(n - 1)", "s <- f(40)", 4, true),
    ];
    struct Dir(std::path::PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Dir(std::env::temp_dir().join(format!("robigo-freeze-steady-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let dir = &dir.0;
    let passes = |file: &std::path::Path, jit: &str| -> u64 {
        let (out, err, status) = boot(
            &[
                std::ffi::OsStr::new("--vanilla"),
                std::ffi::OsStr::new("--quiet"),
                std::ffi::OsStr::new("--no-echo"),
                std::ffi::OsStr::new("-f"),
                file.as_os_str(),
            ],
            // the freeze's check at every collection too: with no pass in
            // steady state, a missed note would leave frozen garbage for the
            // session, which I2 reports (Opus step 2 r7 R3)
            &[
                ("ROBIGO_GC_FREEZE", "1".as_ref()),
                ("ROBIGO_GC_FREEZE_CHECK", "1".as_ref()),
                ("ROBIGO_GC_STATS", "1".as_ref()),
                ("R_ENABLE_JIT", jit.as_ref()),
                ("ROBIGO_GCTORTURE", "0".as_ref()),
            ],
        );
        assert!(status == Some(0) && out.contains("ok"), "{}: {out}\n{err}", file.display());
        let line = err
            .lines()
            .find(|l| l.contains("frozen-reachability passes"))
            .unwrap_or_else(|| panic!("no freeze report: {err}"));
        let n = line.split(", ").find(|p| p.ends_with("frozen-reachability passes")).and_then(|p| p.split(' ').next());
        n.and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("unreadable: {line}"))
    };
    // The steady rounds: 5, and 1 with debug assertions on (the
    // debug-assertions shard's 5-minute budget: the five rounds took boot_db
    // from 0.7 s to 25-38 s there). One round still fails on any write that
    // notes each time the body runs, per iteration or per round: S3's memo,
    // cycling 400 generics through 256 entries, refills every round. Only
    // the release run's five rounds see a source with a longer period (none
    // is known). Measured on the debug-assertions binary, seconds per shape
    // (both JIT levels, 0 and ROUNDS rounds), on a 24-CPU workstation:
    // dispatch 0.5, conditions 1.6, apply 0.9, environments 0.6, jit 0.3,
    // deep frames under gctorture 10.7; 14.7 s in all, against 40.4 s with
    // five rounds and the boot under gctorture.
    const ROUNDS: u32 = if cfg!(debug_assertions) { 1 } else { 5 };
    for &(shape, setup, body, n, torture) in SHAPES {
        let file = |rounds: u32| {
            let f = dir.join(format!("{shape}-{rounds}.R"));
            std::fs::write(
                &f,
                format!("{setup}\ng <- function() {{ for (i in 1:{n}) {{ {body} }}; NULL }}\n{}invisible(gc())\ng()\ninvisible(gc())\nfor (.r in seq_len({rounds})) {{ g(); invisible(gc()) }}\ncat('ok\\n')\n", if torture { "gctorture(TRUE)\n" } else { "" }),
            )
            .unwrap();
            f
        };
        for jit in ["3", "0"] {
            let (warm, steady) = (passes(&file(0), jit), passes(&file(ROUNDS), jit));
            assert_eq!(
                steady,
                warm,
                "{shape} (R_ENABLE_JIT={jit}): {} frozen-reachability passes in {ROUNDS} steady round(s)",
                steady - warm
            );
        }
    }
}
