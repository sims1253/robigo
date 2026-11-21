//! `dyn.load`, `dyn.unload` and the foreign interfaces over robigo-rt
//! (Rdynload.c, dotcode.c), with a package's own shared object
//! (`tests/support/dynload_test.c`, built as packages are) in a session with
//! base loaded:
//!
//! - its `R_init_<pkg>` registers its routines (`R_registerRoutines`), and
//!   `getDLLRegisteredRoutines` lists them;
//! - `.Call`, `.External` and `.External2` find a routine by name (checking
//!   its registered argument count) or through a native symbol reference;
//!   an object that forces symbols (as tools' does) is reached only through
//!   references; a call from a namespace looks in that namespace's DLL;
//! - `dyn.unload` calls `R_unload_<pkg>` as `R_dlsym` finds it and clears
//!   every reference handed out for the object.
//!
//! The object is built twice: `dl_plain`, whose init only registers, and
//! `dl_force`, whose init also turns dynamic lookup off and forces symbols.

#![cfg(all(target_os = "linux", not(miri)))]

mod common;
use common::*;
use robigo_rt::Profile;

const PROFILES: [Profile; 2] = [Profile::Eval, Profile::Bc];

fn plain() -> std::path::PathBuf {
    support_object("dynload_test.c", "dl_plain", &["-DDL_NAME=dl_plain"])
}

fn force() -> std::path::PathBuf {
    support_object("dynload_test.c", "dl_force", &["-DDL_NAME=dl_force", "-DDL_FORCE"])
}

/// `p` as an R string literal.
fn lit(p: &std::path::Path) -> String {
    format!("\"{}\"", p.display().to_string().replace('\\', "\\\\").replace('"', "\\\""))
}

/// The console's output stream since the last call.
fn stdout(s: &mut Session) -> String {
    s.iso.cx().flush_console();
    String::from_utf8_lossy(&s.out.take()).into_owned()
}

/// A session with both objects loaded: `dp` and `df` their "DLLInfo"s.
fn session(p: Profile) -> Session {
    let mut s = Session::new(p);
    let _ = stdout(&mut s);
    s.val(&format!("dp <- dyn.load({})", lit(&plain())));
    assert_eq!(stdout(&mut s), "R_init_dl_plain\n");
    s.val(&format!("df <- dyn.load({})", lit(&force())));
    assert_eq!(stdout(&mut s), "R_init_dl_force\n");
    s
}

fn lgl(x: bool) -> Val {
    Val::Lgl(vec![x as i32])
}

#[test]
fn r_init_registers_the_routines() {
    for p in PROFILES {
        let mut s = session(p);
        // R_registerRoutines turns dynamic lookup on for an object with a
        // handle; dl_force's init then turns it off and forces symbols
        assert_eq!(s.val("dp[[\"dynamicLookup\"]]"), lgl(true), "{p:?}");
        assert_eq!(s.val("dp[[\"forceSymbols\"]]"), lgl(false), "{p:?}");
        assert_eq!(s.val("df[[\"dynamicLookup\"]]"), lgl(false), "{p:?}");
        assert_eq!(s.val("df[[\"forceSymbols\"]]"), lgl(true), "{p:?}");
        s.val("r <- getDLLRegisteredRoutines(dp)");
        assert_eq!(
            s.val("names(r)"),
            Val::Str(vec![".C".into(), ".Call".into(), ".Fortran".into(), ".External".into()])
        );
        assert_eq!(
            s.val("names(r$.Call)"),
            Val::Str(vec!["dl_add1".into(), "dl_nargs".into(), "dl_force_now".into()]),
            "{p:?}"
        );
        assert_eq!(
            s.val("names(r$.External)"),
            Val::Str(vec!["dl_ext".into(), "dl_ext2".into(), "dl_ext2_error".into()]),
            "{p:?}"
        );
        assert_eq!(s.val("sapply(r$.Call, `[[`, \"numParameters\")"), Val::Int(vec![1, -1, 0]), "{p:?}");
        assert_eq!(s.val("r$.External$dl_ext2$numParameters"), int(2), "{p:?}");
        assert_eq!(s.val("r$.C$dl_c$numParameters"), int(1), "{p:?}");
        assert_eq!(s.val("length(r$.Fortran)"), int(0), "{p:?}");
        assert_eq!(s.val("class(r$.Call$dl_add1)"), Val::Str(vec!["CallRoutine".into(), "NativeSymbolInfo".into()]));
        // a path loaded already is not initialized again
        s.val(&format!("dyn.load({})", lit(&plain())));
        assert_eq!(stdout(&mut s), "", "{p:?}");
    }
}

#[test]
fn call_by_name_checks_the_registered_argument_count() {
    for p in PROFILES {
        let mut s = session(p);
        assert_eq!(s.val(".Call(\"dl_add1\", 41L, PACKAGE = \"dl_plain\")"), int(42), "{p:?}");
        assert_eq!(s.val(".Call(\"dl_add1\", 41L)"), int(42), "{p:?}");
        assert_eq!(
            s.err(".Call(\"dl_add1\", 1L, 2L, PACKAGE = \"dl_plain\")"),
            "Incorrect number of arguments (2), expecting 1 for 'dl_add1'",
            "{p:?}"
        );
        assert_eq!(s.val(".Call(\"dl_nargs\", 1, 2, 3, PACKAGE = \"dl_plain\")"), int(3), "{p:?}");
        // not registered: dlsym's, while the object allows dynamic lookup
        assert_eq!(s.val(".Call(\"dl_unregistered\", PACKAGE = \"dl_plain\")"), str1("unregistered"), "{p:?}");
    }
}

/// Lookup settings changed after calls by name (robigo caches `.Call`'s
/// lookups; GNU R on Linux looks up every call): the next call by name
/// sees them.
#[test]
fn a_later_lookup_setting_holds_for_the_next_call() {
    for p in PROFILES {
        let mut s = session(p);
        assert_eq!(s.val(".Call(\"dl_add1\", 1L, PACKAGE = \"dl_plain\")"), int(2), "{p:?}");
        assert_eq!(s.val(".Call(\"dl_force_now\", PACKAGE = \"dl_plain\")"), Val::Null, "{p:?}");
        assert_eq!(s.val("getLoadedDLLs()[[\"dl_plain\"]][[\"forceSymbols\"]]"), lgl(true), "{p:?}");
        assert_eq!(
            s.err(".Call(\"dl_add1\", 1L, PACKAGE = \"dl_plain\")"),
            "\"dl_add1\" not available for .Call() for package \"dl_plain\"",
            "{p:?}"
        );
    }
}

#[test]
fn forced_symbols_are_reached_only_through_references() {
    for p in PROFILES {
        let mut s = session(p);
        assert_eq!(
            s.err(".Call(\"dl_add1\", 1L, PACKAGE = \"dl_force\")"),
            "\"dl_add1\" not available for .Call() for package \"dl_force\"",
            "{p:?}"
        );
        assert_eq!(
            s.err(".External2(\"dl_ext2\", 1, 2, PACKAGE = \"dl_force\")"),
            "\"dl_ext2\" not available for .External() for package \"dl_force\"",
            "{p:?}"
        );
        s.val("sym <- getNativeSymbolInfo(\"dl_add1\", df)");
        assert_eq!(s.val(".Call(sym, 1L)"), int(2), "{p:?}");
        assert_eq!(s.val(".Call(sym$address, 1L)"), int(2), "{p:?}");
        s.val("reg <- getNativeSymbolInfo(\"dl_add1\", df, withRegistrationInfo = TRUE)");
        assert_eq!(s.val("class(reg$address)"), str1("RegisteredNativeSymbol"), "{p:?}");
        assert_eq!(s.val(".Call(reg, 1L)"), int(2), "{p:?}");
        assert_eq!(
            s.err(".Call(reg, 1L, 2L)"),
            "Incorrect number of arguments (2), expecting 1 for 'dl_add1'",
            "{p:?}"
        );
        // a .Call routine's registration does not serve .External
        assert_eq!(s.err(".External(reg, 1)"), "NULL value passed as symbol address", "{p:?}");
        assert_eq!(
            s.err(".Call(1, 2)"),
            "first argument must be a string (of length 1) or native symbol reference",
            "{p:?}"
        );
    }
}

#[test]
fn external_gets_the_arguments_as_a_pairlist() {
    for p in PROFILES {
        let mut s = session(p);
        s.val("x <- .External(\"dl_ext\", 1, b = \"two\", PACKAGE = \"dl_plain\")");
        assert_eq!(s.val("x"), Val::List(vec![str1("dl_ext"), Val::Real(vec![1.0]), str1("two")]), "{p:?}");
        assert_eq!(s.val("names(x)"), Val::Str(vec!["".into(), "".into(), "b".into()]), "{p:?}");
        assert_eq!(s.err(".External(\"dl_ext\", PACKAGE = \"\")"), "PACKAGE = \"\" is invalid", "{p:?}");
        assert_eq!(s.err(".External(.NAME = \"dl_ext\")"), "the first argument should not be named", "{p:?}");
    }
}

#[test]
fn external2_gets_the_call_the_builtin_the_arguments_and_the_frame() {
    for p in PROFILES {
        let mut s = session(p);
        s.val(
            "f <- function() { e <- environment(); \
             r <- .External2(\"dl_ext2\", 1, 2, PACKAGE = \"dl_plain\"); \
             c(identical(r[[1]], quote(.External2(\"dl_ext2\", 1, 2, PACKAGE = \"dl_plain\"))), \
               identical(r[[2]], .External2), \
               identical(r[[3]], list(\"dl_ext2\", 1, 2)), \
               identical(r[[4]], e)) }",
        );
        assert_eq!(s.val("f()"), Val::Lgl(vec![1, 1, 1, 1]), "{p:?}");
        assert_eq!(
            s.err(".External2(\"dl_ext2\", 1, 2, 3, PACKAGE = \"dl_plain\")"),
            "Incorrect number of arguments (3), expecting 2 for 'dl_ext2'",
            "{p:?}"
        );
        assert_eq!(s.err(".External2(\"dl_ext2_error\", PACKAGE = \"dl_plain\")"), "dl_ext2_error was called", "{p:?}");
    }
}

#[test]
fn a_call_from_a_namespace_looks_in_its_dll() {
    for p in PROFILES {
        let mut s = session(p);
        // namespaces as R_IsNamespaceEnv sees them: `.__NAMESPACE__.` holds
        // an environment with a character `spec` (and the DLLs)
        s.val(
            "ns <- function(name, dlls) { e <- new.env(); i <- new.env(parent = baseenv()); \
             assign(\"spec\", c(name = name, version = \"1.0\"), envir = i); \
             if (!is.null(dlls)) assign(\"DLLs\", dlls, envir = i); \
             assign(\".__NAMESPACE__.\", i, envir = e); e }",
        );
        s.val("f <- function() .Call(\"dl_add1\", 1L)");
        s.val("environment(f) <- ns(\"withdll\", list(dl_plain = dp))");
        assert_eq!(s.val("f()"), int(2), "{p:?}");
        s.val("environment(f) <- ns(\"nodll\", NULL)");
        assert_eq!(s.err("f()"), "\"dl_add1\" not resolved from current namespace (nodll)", "{p:?}");
        s.val("environment(f) <- ns(\"forced\", list(dl_force = df))");
        assert_eq!(s.err("f()"), "DLL requires the use of native symbols", "{p:?}");
        // GNU R 4.6.2 too: base's namespace has no DLL to look in
        s.val("environment(f) <- .BaseNamespaceEnv");
        assert_eq!(s.err("f()"), "\"dl_add1\" not resolved from current namespace (base)", "{p:?}");
        // PACKAGE wins over the namespace
        s.val("g <- function() .Call(\"dl_add1\", 1L, PACKAGE = \"dl_plain\")");
        s.val("environment(g) <- ns(\"nodll\", NULL)");
        assert_eq!(s.val("g()"), int(2), "{p:?}");
    }
}

#[test]
fn dyn_unload_calls_r_unload_and_clears_the_references() {
    for p in PROFILES {
        let mut s = session(p);
        s.val("s1 <- getNativeSymbolInfo(\"dl_add1\", dp)");
        s.val("s2 <- getNativeSymbolInfo(\"dl_add1\", dp, withRegistrationInfo = TRUE)");
        s.val("r <- getDLLRegisteredRoutines(dp)");
        assert_eq!(s.val(".Call(s1, 1L) + .Call(s2, 1L) + .Call(r$.Call$dl_add1, 1L)"), int(6), "{p:?}");
        s.val("dyn.unload(dp[[\"path\"]])");
        assert_eq!(stdout(&mut s), "R_unload_dl_plain\n", "{p:?}");
        for x in ["s1", "s2", "r$.Call$dl_add1", "s1$address", "s2$address"] {
            assert_eq!(s.err(&format!(".Call({x}, 1L)")), "NULL value passed as symbol address", "{p:?} {x}");
        }
        assert_eq!(s.err("getDLLRegisteredRoutines(dp)"), "NULL value passed for DllInfo", "{p:?}");
        assert_eq!(s.val("\"dl_plain\" %in% names(getLoadedDLLs())"), lgl(false), "{p:?}");
        assert_eq!(
            s.err(".Call(\"dl_add1\", 1L, PACKAGE = \"dl_plain\")"),
            "\"dl_add1\" not available for .Call() for package \"dl_plain\"",
            "{p:?}"
        );
        let path = plain().display().to_string();
        assert_eq!(s.err("dyn.unload(dp[[\"path\"]])"), format!("shared object '{path}' was not loaded"), "{p:?}");
        // the copies the references held are freed once, at the unload
        s.val("rm(s1, s2, r); invisible(gc())");
        // dl_force turned dynamic lookup off: R_dlsym does not find its
        // R_unload_dl_force, which is not called
        s.val("dyn.unload(df[[\"path\"]])");
        assert_eq!(stdout(&mut s), "", "{p:?}");
        // loaded again: initialized again
        s.val(&format!("dp <- dyn.load({})", lit(&plain())));
        assert_eq!(stdout(&mut s), "R_init_dl_plain\n", "{p:?}");
        assert_eq!(s.val(".Call(\"dl_add1\", 1L, PACKAGE = \"dl_plain\")"), int(2), "{p:?}");
    }
}

/// `so`'s handshake (tests/support/dlinit_test.c) on another thread: wait
/// (60 s at most) for the object's constructor or destructor to signal from
/// inside `dlopen` or `dlclose`, read whether the environment's lock is held
/// then, and let it go on. Neither side waits without a bound, so a failure
/// on either never hangs the test.
///
/// It returns once the thread runs: a thread that starts while `dlopen` runs
/// a constructor waits for it, since a new thread registers its
/// thread-locals' destructors with glibc's `__cxa_thread_atexit_impl`, which
/// takes the loader's lock that `dlopen` holds across the constructors.
fn handshake(so: &std::path::Path) -> std::thread::JoinHandle<bool> {
    let ready = std::path::PathBuf::from(format!("{}.ready", so.display()));
    let go = std::path::PathBuf::from(format!("{}.go", so.display()));
    let (started, running) = std::sync::mpsc::channel();
    let t = std::thread::spawn(move || {
        started.send(()).unwrap();
        let t0 = std::time::Instant::now();
        while !ready.exists() {
            assert!(t0.elapsed() < std::time::Duration::from_secs(60), "the object's code never signalled");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // held at each of 20 reads over about 20 ms: another test's brief
        // hold cannot stand in for this one's
        let held = (0..20).all(|_| {
            std::thread::sleep(std::time::Duration::from_millis(1));
            !robigo_rtcore::clib::env_lock_is_free()
        });
        std::fs::write(&go, b"").unwrap();
        held
    });
    running.recv().unwrap();
    t
}

/// The object's markers, removed however the test ends: on a failed assert
/// or an R error (`s.val` panics) the object stays loaded, and without its
/// `.armed` its destructor returns at once at the process's exit instead of
/// waiting 60 s for a `.go`; `.go` is written too, for a handshake still
/// waiting.
struct Disarm {
    so: std::path::PathBuf,
    dir: std::path::PathBuf,
}

impl Drop for Disarm {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(format!("{}.armed", self.so.display()));
        let _ = std::fs::write(format!("{}.go", self.so.display()), b"");
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A library's initializers run inside `dyn.load`'s `dlopen`, and its
/// finalizers inside `dyn.unload`'s `dlclose`, with the environment's lock
/// held shared, so no isolate's `Sys.setenv` writes while they may read it
/// (clib::with_env_shared).
#[test]
fn a_librarys_initializers_and_finalizers_hold_env() {
    // a copy of its own, so that its marker files are this process's
    let built = support_object("dlinit_test.c", "dl_init", &[]);
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("dlinit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let so = dir.join("dl_init.so");
    std::fs::copy(&built, &so).unwrap();
    let _disarm = Disarm { so: so.clone(), dir: dir.clone() };
    std::fs::write(format!("{}.armed", so.display()), b"").unwrap();
    let mut s = Session::new(Profile::Eval);
    // The two phases cannot overlap: the constructor and the destructor run
    // on this thread, inside `dlopen` and `dlclose`, and remove their
    // markers before they return (dlinit_test.c), so once `s.val` returns
    // the next `.ready` can only be the next phase's.
    let t = handshake(&so);
    s.val(&format!("invisible(dyn.load({}))", lit(&so)));
    assert!(t.join().unwrap(), "the constructor ran without the environment's lock");
    let t = handshake(&so);
    s.val(&format!("dyn.unload({})", lit(&so)));
    assert!(t.join().unwrap(), "the destructor ran without the environment's lock");
}
