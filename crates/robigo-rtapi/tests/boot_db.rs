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
    boot_bin(std::path::Path::new(env!("CARGO_BIN_EXE_robigo-rt")), args, env)
}

/// [`boot`] with the robigo-rt at `bin` (its R_HOME the install layout's:
/// `lib/robigo` beside its `bin`).
fn boot_bin(
    bin: &std::path::Path,
    args: &[&std::ffi::OsStr],
    env: &[(&str, &std::ffi::OsStr)],
) -> (String, String, Option<i32>) {
    // one directory per run (the tests run in parallel)
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!("robigo-boot-db-home-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let o = Command::new(bin)
        .args(args)
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
        // the test's own, last (over the removals above)
        .envs(env.iter().copied())
        .output()
        .unwrap_or_else(|e| panic!("running robigo-rt: {e}"));
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

/// An unknown `ROBIGO_RT_BOOT` (a typo) is an error, not M1's loader by
/// default; empty is unset.
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
        ("", "robigo-rt: ROBIGO_RT_BOOT is 'rbd': it is 'stage0', 'rdb' or unset (M1's loader)\n", Some(2))
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
