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
}

/// eval.c's `check_stack_balance` after a `.Call` that leaves an object
/// protected: GNU R's warning, with robigo's protect-stack depths (the C
/// API's stack holds only C's protections: GNU R's depth at the call
/// includes its evaluator's own, which robigo does not keep).
#[test]
fn an_unbalanced_routine_gets_the_stack_imbalance_warning() {
    let mut s = Session::new(Profile::Eval);
    s.stderr();
    s.val("invisible(.Call(\"rt_stash\"))");
    assert_eq!(s.stderr(), "");
    s.val("invisible(.Call(\"rt_leave_protected\"))");
    assert_eq!(s.stderr(), "Warning: stack imbalance in '.Call', 0 then 1\n");
}
