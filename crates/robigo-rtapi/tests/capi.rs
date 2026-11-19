//! The C API over robigo-rt (docs/g3.md §3 item 6, §6.5, §11): package C
//! code (`tests/support/capi_test.c`, built as a package is) called through
//! `.Call` in a session with base loaded.
//!
//! - E8 across a C frame, in both profiles: an `on.exit` that catches its
//!   own error during an unwind leaves the outer condition; an
//!   `R_UnwindProtect` cleanup that catches an error resumes the original
//!   continuation; a C context's `cend` runs before the jump.
//! - Re-entry: native code calling R calling native code, collecting at
//!   every level; cpp11's cleanup that leaves `R_UnwindProtect` by its own
//!   `longjmp`; misuse across isolates and from outside any isolate.
//! - L8: experiment A's probes (`pin_premature`, `pin_extbuf`) under
//!   gctorture, and the paths GNU R serves without allocating, which must
//!   not collect.
//! - The exports themselves: every one that may allocate is entered through
//!   a machine entry, every one is a dynamic symbol, and `api.c` compiles
//!   against GNU R 4.6.2's own declarations.

#![cfg(all(target_os = "linux", not(miri)))]
#![allow(clippy::undocumented_unsafe_blocks)]

mod common;
use common::*;
use robigo_rt::Profile;
use robigo_rtapi::{EXPORTS, Kind};

const PROFILES: [Profile; 2] = [Profile::Eval, Profile::Bc];

// ---- the exports ------------------------------------------------------------

#[test]
fn api_c_reads_the_heaps_layout() {
    robigo_rtapi::check_layout();
}

/// `api.c`'s definitions against GNU R 4.6.2's declarations: compiled with
/// robigo's `Rinternals.h` (GNU R's, then the globals' macros) forced in, a
/// definition whose type differs from GNU R's prototype does not compile.
#[test]
fn api_c_defines_gnu_rs_declarations() {
    let api = root().join("crates/robigo-rtapi/src/api.c");
    let st = std::process::Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args(["-fsyntax-only", "-std=gnu11", "-Werror", "-DR_NO_REMAP"])
        .args(include_flags())
        .args(["-include", "R.h", "-include", "Rinternals.h", "-include", "R_ext/Rdynload.h"])
        .arg(&api)
        .status()
        .unwrap();
    assert!(st.success(), "api.c disagrees with GNU R's headers");
}

/// The foreman's rule (H2): every export that may collect or exit is
/// entered through a machine entry, whose first instructions push the
/// callee-saved registers C left in place. GNU R's allocating functions are
/// all such entries, or C whose collecting paths are.
#[test]
fn every_allocating_export_has_a_machine_entry() {
    let mut names = std::collections::HashSet::new();
    for e in EXPORTS {
        assert!(names.insert(e.name), "{} is listed twice", e.name);
        if e.gnu_allocates {
            assert!(e.kind != Kind::Leaf, "{} allocates in GNU R: not a leaf", e.name);
        }
        if let Kind::Entry(n) = e.kind {
            assert!(n <= 5, "{}: class {n}", e.name);
            #[cfg(target_arch = "x86_64")]
            {
                // push rbp; push rbx; push r12; push r13; push r14; push r15
                const PROLOGUE: [u8; 10] = [0x55, 0x53, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57];
                let code = unsafe { std::slice::from_raw_parts(e.addr as *const u8, PROLOGUE.len()) };
                assert_eq!(code, PROLOGUE, "{} is not entered through a machine entry", e.name);
            }
        }
    }
    // the ones the capi cases and the probes call
    for n in [
        "Rf_allocVector",
        "Rf_cons",
        "Rf_lang2",
        "Rf_setAttrib",
        "Rf_eval",
        "R_tryEval",
        "Rf_install",
        "Rf_mkChar",
        "R_RegisterCFinalizerEx",
        "R_MakeWeakRef",
        "R_UnwindProtect",
        "Rf_error",
    ] {
        assert!(names.contains(n), "{n} is not exported");
    }
}

/// The binaries that link the C API export it (the test binary, like
/// `robigo-rt`, links with `-rdynamic`): `dlsym` finds each export at its
/// address.
#[test]
fn every_export_is_a_dynamic_symbol() {
    for e in EXPORTS {
        let name = std::ffi::CString::new(e.name).unwrap();
        let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
        assert_eq!(p as *const (), e.addr, "{} is not exported from the binary", e.name);
    }
}

// ---- E8 across a C frame ---------------------------------------------------------

/// E8 1: `f`'s `on.exit` code catches an error of its own during the unwind
/// of `f`'s error; the error that leaves is the outer one, across the C
/// frame that evaluated `f`.
#[test]
fn e8_an_on_exit_that_catches_its_own_error_leaves_the_outer_condition_across_a_c_frame() {
    for p in PROFILES {
        let mut s = Session::new(p);
        s.val("f <- function() { on.exit(.Call(\"rt_catch_inner\")); .Call(\"rt_error\", \"outer\") }");
        assert_eq!(s.err(".Call(\"rt_eval_call\", f)"), "outer", "{p:?}");
        assert_eq!(s.log(), ["inner caught"], "{p:?}");
    }
}

/// E8 2: the cleanup of an `R_UnwindProtect` catches an error of its own;
/// the original exit resumes and reaches the catcher.
#[test]
fn e8_an_unwind_protect_cleanup_that_catches_resumes_the_original_continuation() {
    for p in PROFILES {
        let mut s = Session::new(p);
        assert_eq!(s.err(".Call(\"rt_unwind_catching_cleanup\")"), "original", "{p:?}");
        assert_eq!(s.log(), ["cleanup: jump", "cleanup: caught its own"], "{p:?}");
    }
}

/// E8 3: `R_ExecWithCleanup`'s cleanup (its `CTXT_CCODE` record's `cend`)
/// runs before the jump, while the routine's frame is still there.
#[test]
fn e8_a_c_contexts_cend_runs_before_the_jump() {
    for p in PROFILES {
        let mut s = Session::new(p);
        assert_eq!(s.err(".Call(\"rt_exec_with_cleanup_fails\")"), "from the body", "{p:?}");
        assert_eq!(s.log(), ["cleanup: before the jump"], "{p:?}");
    }
}

// ---- re-entry --------------------------------------------------------------------

/// C calls R calls C, 30 deep, each level holding a vector only in C
/// locals and the protect stack across a full collection (`R_gc`) and
/// the levels below; under gctorture every allocating export collects too.
#[test]
fn nested_native_reentry_keeps_every_levels_objects() {
    for torture in [false, true] {
        let mut s = Session::new(Profile::Eval);
        s.val("g <- function(n) .Call(\"rt_nested\", g, n)");
        if torture {
            s.cx().gc_torture(1, 0);
        }
        // each level sums 0..63 = 2016
        assert_eq!(s.val("g(30L)"), int(2016 * 31), "torture {torture}");
        s.cx().gc_torture(0, 0);
        assert_eq!(s.cx().pp_top(), 0, "the protect stack is balanced");
    }
}

/// `vapply` copies each value FUN returns into its answer as it comes; a
/// package ALTREP value (which GNU R reads through its methods, and
/// robigo-rt does not materialize yet) is robigo's error, not a panic, in
/// both profiles, through the closure and the `.Internal`.
#[test]
fn vapply_of_a_package_altrep_result_is_not_implemented_yet() {
    for p in PROFILES {
        let mut s = Session::new(p);
        s.val("f <- function(i) .Call(\"rt_altrep_one\"); g <- function(...) .Internal(vapply(1:2, f, 0L, FALSE))");
        for src in ["vapply(1:2, f, 0L)", "g()"] {
            let m = s.err(src);
            assert!(m.contains("vapply() of a package ALTREP result of FUN"), "{p:?} {src}: {m}");
        }
    }
}

/// Printing or deparsing a package ALTREP vector (its length and elements
/// are its C methods', M5's) is robigo's refusal, raised before the
/// printers read standard storage: not a panic, in both profiles, at the
/// top and inside a list.
#[test]
fn printing_a_package_altrep_vector_is_not_implemented_yet() {
    for p in PROFILES {
        let mut s = Session::new(p);
        s.val("z <- .Call(\"rt_altrep_str\", c(\"ff\", \"10\"))");
        for (src, want) in [
            ("print(z)", "printing a package ALTREP vector is not implemented yet"),
            ("print(list(a = z))", "printing a package ALTREP vector is not implemented yet"),
            ("deparse(z)", "deparse of a package ALTREP vector is not implemented yet"),
            ("cat(z)", "cat() of a package ALTREP vector is not implemented yet"),
            ("cat(\"x\", fill = 20, labels = z)", "cat() of a package ALTREP vector is not implemented yet"),
            ("cat(\"x\", \"y\", sep = z)", "cat() of a package ALTREP vector is not implemented yet"),
            (
                ".Internal(encodeString(z, 0L, \"\", 0L, TRUE))",
                "encodeString() of a package ALTREP vector is not implemented yet",
            ),
            // as an attribute's value (names and dimnames go through the same
            // check, print_value_rec's and deparse2buff's attribute walk)
            ("x <- c(1, 2); attr(x, \"w\") <- z; print(x)", "printing a package ALTREP vector is not implemented yet"),
            (
                "x <- c(1, 2); attr(x, \"w\") <- z; deparse(x)",
                "deparse of a package ALTREP vector is not implemented yet",
            ),
            // a function's source text (useSource)
            (
                "f <- quote(function(x) x); f[[4]] <- z; deparse(f, control = \"useSource\")",
                "deparse of a package ALTREP vector is not implemented yet",
            ),
        ] {
            let m = s.err(src);
            assert!(m.contains(want), "{p:?} {src}: {m}");
        }
    }
}

/// The attribute setters read a package ALTREP vector's length through its
/// class's Length method (GNU R's `xlength`, `ALTREP_LENGTH`), so `names<-`,
/// `dim<-`, `dimnames<-`, `comment<-` and `row.names` take one as GNU R
/// does; where GNU R would read its elements, robigo says it cannot.
#[test]
fn the_attribute_setters_read_a_package_altrep_length() {
    for p in PROFILES {
        let mut s = Session::new(p);
        // production identities: a setter writes an unshared package
        // ALTREP vector in place (under ROBIGO_NO_INPLACE it would copy it,
        // through its Duplicate method, which robigo refuses)
        s.cx().set_no_inplace(false);
        s.val("z <- .Call(\"rt_altrep_str\", c(\"ff\", \"10\"))");
        s.val("zi <- .Call(\"rt_altrep_int\", 5:6); zr <- .Call(\"rt_altrep_real\", c(1, 2, 1))");
        s.val("zl <- .Call(\"rt_altrep_list\", list(c(\"a\", \"b\"), NULL))");
        for (src, want) in [
            ("x <- c(1, 2); names(x) <- z; length(names(x))", int(2)),
            ("x <- c(1, 2); attr(x, \"names\") <- z; length(names(x))", int(2)),
            ("x <- c(1, 2); dim(x) <- 2L; names(x) <- z; length(dimnames(x)[[1]])", int(2)),
            ("m <- matrix(1:4, 2); dimnames(m) <- list(z, NULL); length(dimnames(m)[[1]])", int(2)),
            ("x <- c(1, 2); comment(x) <- z; length(attr(x, \"comment\"))", int(2)),
            ("x <- c(1, 2); class(x) <- character(0); length(x)", int(2)),
            ("d <- list(1:2); attr(d, \"row.names\") <- zi; length(attr(d, \"row.names\"))", int(2)),
            ("y <- .Call(\"rt_altrep_str\", c(\"ff\", \"10\")); names(y) <- c(\"a\", \"b\"); length(names(y))", int(2)),
            ("y <- .Call(\"rt_altrep_str\", c(\"ff\", \"10\")); dim(y) <- 2L; dim(y)", int(2)),
        ] {
            assert_eq!(s.val(src), want, "{p:?} {src}");
        }
        for (src, want) in [
            // the case the print refusal could not reach before
            (
                "dn <- list(c(\"a\", \"b\")); names(dn) <- .Call(\"rt_altrep_str\", \"axis\"); \
                 x <- 1:2; dim(x) <- 2L; dimnames(x) <- dn; print(x)",
                "printing a package ALTREP vector is not implemented yet",
            ),
            (
                "dn <- list(c(\"a\", \"b\"), NULL); names(dn) <- z; \
                 m <- matrix(1:4, 2); dimnames(m) <- dn; print(m)",
                "printing a package ALTREP vector is not implemented yet",
            ),
            ("x <- c(1, 2); attr(x, \"names\") <- pairlist(z)", "incompatible 'names' argument"),
            ("x <- c(1, 2); names(x) <- z; print(x)", "printing a package ALTREP vector is not implemented yet"),
            ("x <- c(1, 2); names(x) <- z; deparse(x)", "deparse of a package ALTREP vector is not implemented yet"),
            (
                "m <- matrix(1:4, 2); dimnames(m) <- list(z, NULL); print(m)",
                "printing a package ALTREP vector is not implemented yet",
            ),
            // GNU R reads the elements
            ("x <- c(1, 2); attr(x, \"names\") <- zi", "coercing a package ALTREP vector"),
            ("m <- matrix(1:4, 2); dimnames(m) <- list(zi, NULL)", "coercing a package ALTREP vector"),
            ("x <- c(1, 2, 3); names(x) <- z", "recycling package ALTREP names is not implemented yet"),
            ("x <- pairlist(a = 1, b = 2); names(x) <- z", "package ALTREP names of a pairlist or call"),
            ("x <- c(1, 2); class(x) <- z", "a package ALTREP class attribute is not implemented yet"),
            ("m <- matrix(1:4, 2); dimnames(m) <- zl", "a package ALTREP list as dimnames is not implemented yet"),
            ("x <- 1:3; attr(x, \"tsp\") <- zr", "a package ALTREP 'tsp' attribute is not implemented yet"),
            (
                "d <- list(1:2); attr(d, \"row.names\") <- .Call(\"rt_altrep_real\", c(NA, -2))",
                "row names from a package ALTREP double vector is not implemented yet",
            ),
            // a shared one is copied first (its Duplicate method)
            ("y <- z; names(y) <- c(\"a\", \"b\")", "'duplicate' of an ALTREP object is not implemented yet"),
            // the length GNU R reads, not 1
            ("x <- 1; names(x) <- z", "'names' attribute [2] must be the same length as the vector [1]"),
            (
                "y <- .Call(\"rt_altrep_str\", c(\"ff\", \"10\")); dim(y) <- 3L",
                "dims [product 3] do not match the length of object [2]",
            ),
            ("x <- 1:2; attr(x, \"tsp\") <- zi", "'tsp' attribute must be numeric of length three"),
        ] {
            let m = s.err(src);
            assert!(m.contains(want), "{p:?} {src}: {m}");
        }
    }
}

/// The setters run a package class's Length method as many times as GNU R
/// 4.6.2 does (counts from the oracle with the same class compiled as a
/// package: `R CMD SHLIB`, a counter in the Length method).
#[test]
fn the_attribute_setters_call_the_length_method_as_gnu_r_does() {
    for p in PROFILES {
        let mut s = Session::new(p);
        // production identities: a setter writes an unshared package
        // ALTREP vector in place (under ROBIGO_NO_INPLACE it would copy it,
        // through its Duplicate method, which robigo refuses)
        s.cx().set_no_inplace(false);
        for (setup, assign, calls) in [
            ("x <- c(1, 2); z <- .Call(\"rt_altrep_str\", c(\"a\", \"b\"))", "names(x) <- z", 2),
            ("x <- 1; z <- .Call(\"rt_altrep_str\", c(\"a\", \"b\"))", "names(x) <- z", 3),
            ("y <- .Call(\"rt_altrep_str\", c(\"a\", \"b\"))", "names(y) <- c(\"u\", \"v\")", 2),
            ("y <- .Call(\"rt_altrep_str\", c(\"a\", \"b\"))", "attr(y, \"names\") <- pairlist(\"u\", \"v\")", 6),
            ("y <- .Call(\"rt_altrep_str\", c(\"a\", \"b\"))", "dim(y) <- 2L", 1),
            ("m <- matrix(1:4, 2); z <- .Call(\"rt_altrep_str\", c(\"a\", \"b\"))", "dimnames(m) <- list(z, NULL)", 2),
            ("m <- matrix(1:6, 3); z <- .Call(\"rt_altrep_str\", c(\"a\", \"b\"))", "dimnames(m) <- list(z, NULL)", 2),
            ("x <- 1; z <- .Call(\"rt_altrep_str\", c(\"a\", \"b\"))", "comment(x) <- z", 2),
            ("d <- list(1:2); z <- .Call(\"rt_altrep_int\", 5:6)", "attr(d, \"row.names\") <- z", 0),
            ("x <- 1:2; z <- .Call(\"rt_altrep_int\", 5:6)", "attr(x, \"tsp\") <- z", 1),
            // isVectorizable stops at the first element longer than 1
            ("x <- c(1, 2); z <- .Call(\"rt_altrep_str\", c(\"a\", \"b\"))", "attr(x, \"names\") <- pairlist(z, z)", 1),
        ] {
            s.val(setup);
            let v = if setup.starts_with("y <-") { "y" } else { "z" };
            s.val(&format!(".Call(\"rt_altrep_length_calls\", {v})"));
            // (an error or not: GNU R's count is the same either way; NULL
            // last, so that describing the result reads no length)
            let _ = s.run(&format!("{assign}; NULL"));
            assert_eq!(
                s.val(&format!(".Call(\"rt_altrep_length_calls\", {v})")),
                int(calls),
                "{p:?} {setup}; {assign}"
            );
        }
    }
}

/// cpp11's `unwind_protect`: the cleanup leaves `R_UnwindProtect` by its
/// own `longjmp` (past the export's frame), the routine resumes the unwind
/// with `R_ContinueUnwind`, and the error reaches the catcher; the protect
/// stack and the C segments are as the routine left them, so later
/// collections read only live segments.
#[test]
fn cpp11s_cleanup_longjmp_past_r_unwind_protect() {
    for p in PROFILES {
        let mut s = Session::new(p);
        s.val("token <- .Call(\"rt_make_token\")");
        for i in 0..20 {
            assert_eq!(s.err(&format!(".Call(\"rt_cpp11\", token, \"cpp11 {i}\")")), format!("cpp11 {i}"), "{p:?}");
            assert_eq!(s.log(), ["longjmp landed, stack as before"], "{p:?}");
            assert_eq!(s.val(".Call(\"rt_collect\")"), int(1));
        }
        assert_eq!(s.cx().pp_top(), 0);
    }
}

// ---- L8 ----------------------------------------------------------------------------

/// Experiment A's premature-finalization probe: an external pointer held
/// only by a C local, passed to an allocating export under gctorture, is not
/// finalized during the call (GNU R 4.6.2: FALSE for all seven).
#[test]
fn pin_premature_no_argument_is_finalized_during_an_export() {
    for p in PROFILES {
        let mut s = Session::new(p);
        s.val("renv <- new.env()");
        s.cx().gc_torture(1, 0);
        let v = s.val(".Call(\"capi_pp_probe\", renv)");
        s.cx().gc_torture(0, 0);
        assert_eq!(v, Val::Lgl(vec![0; 7]), "{p:?}");
    }
}

/// Experiment A's external-buffer probe: an external pointer held only by a
/// C local is finalized at a full collection during the call, as GNU R
/// does (both TRUE).
#[test]
fn pin_extbuf_is_finalized_as_gnu_r_finalizes_it() {
    let mut s = Session::new(Profile::Eval);
    assert_eq!(s.val(".Call(\"capi_eb_probe\")"), Val::Lgl(vec![1, 1]));
}

/// L8's allocation parity, by experiment A's method: an external pointer
/// held only by a C local is finalized if, and only if, the export called
/// next collects (one is due). GNU R allocates on none of these paths, so
/// none may collect here; a new name, a cons and `R_tryEval` of a constant
/// (its `R_PreserveObject` of the value) do allocate.
#[test]
fn paths_gnu_r_serves_without_allocating_never_collect() {
    let mut s = Session::new(Profile::Eval);
    s.val(
        "bound <- 1; v <- c(a = 1); d <- structure(list(), row.names = c(\"a\", \"b\")); n <- \"no.one.has.this.name\"",
    );
    s.cx().gc_torture(1, 0);
    for (what, x) in [
        ("install known", "1"),
        ("mkChar cached", "1"),
        ("ScalarLogical", "1"),
        ("setAttrib existing", "v"),
        ("getAttrib row.names", "d"),
        ("getAttrib absent", "v"),
        ("defineVar bound", "v"),
        ("PROTECT", "v"),
    ] {
        assert_eq!(
            s.val(&format!(".Call(\"rt_collects_in\", \"{what}\", {x})")),
            Val::Lgl(vec![0]),
            "{what} collected"
        );
    }
    for (what, x) in [("new name", "n"), ("cons", "v"), ("tryEval constant", "1L")] {
        assert_eq!(s.val(&format!(".Call(\"rt_collects_in\", \"{what}\", {x})")), Val::Lgl(vec![1]), "{what} did not");
    }
    s.cx().gc_torture(0, 0);
}

/// An `R_alloc` block is C's: its address stays valid across a full
/// collection (under `ROBIGO_GC_MOVE=1` the debug collector must not move
/// it).
#[test]
fn an_r_alloc_block_keeps_its_address_across_a_collection() {
    let mut s = Session::new(Profile::Eval);
    assert_eq!(s.val(".Call(\"rt_r_alloc_survives\")"), Val::Lgl(vec![1]));
}

// ---- misuse --------------------------------------------------------------------------

/// Run this test binary's test `name` alone in a child with `env` set; its
/// stderr and whether it succeeded.
fn child(name: &str) -> (bool, String) {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env("ROBIGO_RTAPI_CHILD", "1")
        .output()
        .unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn in_child() -> bool {
    std::env::var_os("ROBIGO_RTAPI_CHILD").is_some()
}

/// Child: isolate A's object handed by C to an export of isolate B.
#[test]
fn child_cross_isolate_object() {
    if !in_child() {
        return;
    }
    let mut a = Session::new(Profile::Eval);
    let mut b = Session::new(Profile::Eval);
    a.val(".Call(\"rt_stash\")");
    b.val(".Call(\"rt_use_stash\")");
}

/// An object of one isolate rooted by an export running in another is
/// caught in release builds, and stops the process (an export cannot
/// unwind into C).
#[test]
fn an_object_of_another_isolate_is_refused_by_the_exports() {
    if in_child() {
        return;
    }
    let (ok, err) = child("child_cross_isolate_object");
    assert!(!ok, "the misuse went unnoticed");
    assert!(err.contains("robigo: an object of another isolate"), "{err}");
}

/// Child: an export called on a thread where no isolate runs C code.
#[test]
fn child_export_without_isolate() {
    if !in_child() {
        return;
    }
    let so = std::ffi::CString::new(support_so().display().to_string()).unwrap();
    let h = unsafe { libc::dlopen(so.as_ptr(), libc::RTLD_NOW) };
    assert!(!h.is_null());
    let f = unsafe { libc::dlsym(h, c"rt_outside".as_ptr()) };
    assert!(!f.is_null(), "rt_outside is not exported from the support library");
    let f: extern "C" fn() -> usize = unsafe { std::mem::transmute(f) };
    std::thread::spawn(move || f()).join().unwrap();
}

#[test]
fn an_export_outside_any_isolate_aborts_with_a_message() {
    if in_child() {
        return;
    }
    let (ok, err) = child("child_export_without_isolate");
    assert!(!ok);
    assert!(err.contains("no isolate runs C code"), "{err}");
}

// ---- errors, warnings, the protect stack -----------------------------------------------

#[test]
fn accessor_errors_are_gnu_rs() {
    let mut s = Session::new(Profile::Eval);
    assert_eq!(
        s.err(".Call(\"rt_integer_of\", \"a\")"),
        "INTEGER() can only be applied to a 'integer', not a 'character'"
    );
    assert_eq!(s.err(".Call(\"rt_over_unprotect\")"), "unprotect(): only 0 protected items");
    assert_eq!(s.err(".Call(\"nope\")"), "C symbol name \"nope\" not in load table");
    assert_eq!(
        s.err(".Call(\"rt_noop3\", 1, 2, 3, PACKAGE = \"other\")"),
        "\"rt_noop3\" not available for .Call() for package \"other\""
    );
    assert_eq!(s.val(".Call(\"rt_warn\")"), int(1));
    // dotcode.c's MAX_ARGS: a 66th argument is GNU R's error, raised
    // before the native entry
    let many = vec!["1"; 66].join(", ");
    assert_eq!(s.err(&format!(".Call(\"rt_noop3\", {many})")), "too many arguments in foreign function call");
    assert_eq!(s.val(&format!(".Call(\"rt_noop3\", {})", vec!["1"; 65].join(", "))), Val::Null);
}

/// `coerceVector` to a type code no SEXPTYPE has, past robigo's `u8`
/// codes: COERCE_ERROR names the code the caller gave, after type2char's
/// warning (GNU R reads past its type table for such a code, so no oracle
/// case records it).
#[test]
fn coerce_vector_to_an_unknown_type_names_the_callers_code() {
    let mut s = Session::new(Profile::Eval);
    s.stderr();
    assert_eq!(s.val(".Call(\"rt_coerce_to\", 1:2, 14)"), Val::Real(vec![1.0, 2.0]));
    assert_eq!(
        s.val(
            "w <- NULL; tryCatch(withCallingHandlers(.Call(\"rt_coerce_to\", 1:2, 999), \
             warning = function(c) { w <<- conditionMessage(c); invokeRestart(\"muffleWarning\") }), \
             error = function(e) c(w, conditionMessage(e)))"
        ),
        Val::Str(vec![
            "type 999 is unimplemented in 'type2char'".into(),
            "cannot coerce type 'integer' to vector of type 'unknown type #999'".into(),
        ])
    );
    // the warning was muffled and the error caught: nothing reached the console
    assert_eq!(s.stderr(), "");
}

/// eval.c's `check_stack_balance` after a `.Call` that leaves an object
/// protected: GNU R's warnings with GNU R's depths, the `.Call`'s and then
/// the enclosing builtin's, for an `eval` of the call at the top level
/// with nothing protected (an embedder's `Rf_eval`; Rscript's REPL
/// protects its statement first, tests/rt/capi/ppacct-*.R).
#[test]
fn an_unbalanced_routine_gets_the_stack_imbalance_warning() {
    let mut s = Session::new(Profile::Eval);
    s.stderr();
    s.val("invisible(.Call(\"rt_stash\"))");
    assert_eq!(s.stderr(), "");
    s.val("invisible(.Call(\"rt_leave_protected\"))");
    assert_eq!(
        s.stderr(),
        "Warning: stack imbalance in '.Call', 2 then 3\nWarning: stack imbalance in 'invisible', 1 then 2\n"
    );
}

/// `R_UnwindProtect` with `R_NilValue` for its continuation, which is not
/// the `NULL` that asks for one (context.c): GNU R calls `fun`, then its
/// `SETCAR(cont, result)` fails with "bad value", and the cleanup never
/// runs.
#[test]
fn unwind_protect_with_r_nil_value_is_setcars_bad_value() {
    let mut s = Session::new(Profile::Eval);
    s.log();
    assert_eq!(s.err(".Call(\"rt_unwind_nil\")"), "bad value");
    assert_eq!(s.log(), ["fun"]);
}

/// `error()` formats into `min(BUFSIZE, R_WarnLength)` bytes, and
/// `R_WarnLength` is `options(warning.length)`: 999 characters at the
/// default 1000, all 1500 at 2000.
#[test]
fn errors_from_c_are_bounded_by_warning_length() {
    let mut s = Session::new(Profile::Eval);
    assert_eq!(s.err(".Call(\"rt_error_n\", 1500L)").len(), 999);
    s.val("invisible(options(warning.length = 2000L))");
    assert_eq!(s.err(".Call(\"rt_error_n\", 1500L)").len(), 1500);
}

/// `Rprintf` of a text longer than its buffer writes all of it.
#[test]
fn rprintf_writes_a_long_text_whole() {
    let mut s = Session::new(Profile::Eval);
    s.cx().flush_console();
    s.out.take();
    s.val("invisible(.Call(\"rt_print_n\", 20000L))");
    s.cx().flush_console();
    let out = s.out.take();
    assert_eq!(out.len(), 20000);
    assert!(out.iter().all(|&b| b == b'x'));
}

/// An index matrix that is a package ALTREP vector (integer, double or
/// character) is refused before its elements are read, as `make_subscript`
/// refuses any package ALTREP subscript: in `x[m]` and in `x[m] <- v`.
#[test]
fn a_package_altrep_index_matrix_is_not_implemented_yet() {
    for p in PROFILES {
        let mut s = Session::new(p);
        // production identities: `dim<-` writes an unshared package ALTREP
        // vector in place (under ROBIGO_NO_INPLACE it would copy it, through
        // its Duplicate method, which robigo refuses)
        s.cx().set_no_inplace(false);
        s.val("x <- matrix(1:4, 2, dimnames = list(c(\"a\", \"b\"), c(\"c\", \"d\")))");
        s.val("mi <- .Call(\"rt_altrep_int\", c(1L, 2L)); dim(mi) <- c(1L, 2L)");
        s.val("mr <- .Call(\"rt_altrep_real\", c(1, 2)); dim(mr) <- c(1L, 2L)");
        s.val("ms <- .Call(\"rt_altrep_str\", c(\"a\", \"d\")); dim(ms) <- c(1L, 2L)");
        for m in ["mi", "mr", "ms"] {
            for src in [format!("x[{m}]"), format!("y <- x; y[{m}] <- 0L")] {
                let e = s.err(&src);
                assert!(
                    e.contains("robigo: subscripting with an ALTREP object is not implemented yet"),
                    "{p:?} {src}: {e}"
                );
            }
        }
    }
}
