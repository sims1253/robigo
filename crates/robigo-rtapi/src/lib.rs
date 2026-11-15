//! robigo-rtapi: GNU R's C API over `robigo-rt` (`docs/g3.md` §3 item 6,
//! §6.5, §11, §13's C-boundary row), with no host table, and the
//! `robigo-rt` binary (`main.rs`), which links it so that the packages it
//! loads find the API.
//!
//! # What an export does
//!
//! - **Allocation parity** (L8): an export may collect only where GNU R's
//!   same function allocates, and then collects on entry when a collection
//!   is due (`Safepoint::Export`), before it allocates. Paths GNU R serves
//!   without allocating allocate nothing here either: `install` of a name
//!   the isolate has, `mkChar` of a cached string, `setAttrib` with an
//!   interned name.
//! - **Rooted arguments** (§3 item 6): an export's `SEXP` arguments are
//!   precise roots for its whole duration ([`entry::CArg`]), so an object
//!   only a C local holds is not finalized during the call (experiment A's
//!   `pin_premature` probe).
//! - **Exits** (§6.5): a Rust `Err` reaching an export runs `R_run_onexits`
//!   semantics, then the export `longjmp`s to the innermost native entry
//!   ([`robigo_rtcore::Landing`]).
//! - **C segments** (§3): an export that may collect is entered through a
//!   machine entry generated per argument-count class ([`entry!`]): the
//!   stub pushes the callee-saved registers C left in place and passes the
//!   lowest one's address, where the export closes its caller's C segment;
//!   no Rust frame is ever scanned. The exports that cannot collect or exit
//!   are plain functions, or C (`api.c`) where their error paths need a
//!   machine entry of their own. [`EXPORTS`] lists every export with its
//!   kind; a test checks that every allocating one is entered through a
//!   stub.
//!
//! The per-isolate C globals (`R_NilValue`, the protect stack, ...) are
//! robigo-rtcore's (`capi.rs`), reached through the C API's one
//! thread-local; robigo's `include/Rinternals.h` makes GNU R's names macros
//! over it, on top of GNU R 4.6.2's public headers.

#![allow(unsafe_code)]

pub mod entry;

mod dotcode;
mod errors;
mod eval;
mod memory;
mod unwind;

pub use dotcode::{DllInfo, loaded_dlls};

use robigo_rtcore::Cx;

/// How an export is entered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Through a machine entry of argument-count class `n` ([`entry!`]): it
    /// may collect or exit.
    Entry(u8),
    /// A plain function that neither collects nor exits (an accessor, a
    /// print), Rust or C.
    Leaf,
    /// C whose only collecting or exiting paths call entries (`api.c`).
    C,
}

/// One export: its C name, its address, how it is entered, and whether
/// GNU R's function of that name may allocate (L8: then it must be an
/// [`Kind::Entry`] or C over entries).
#[derive(Clone, Copy, Debug)]
pub struct Export {
    pub name: &'static str,
    pub addr: *const (),
    pub kind: Kind,
    pub gnu_allocates: bool,
}

// SAFETY: an export's address is a function's: immutable code, shared
// freely between threads (the table is a static).
unsafe impl Sync for Export {}

macro_rules! exports {
    ($($kind:expr, $alloc:expr => $($f:ident)*;)*) => {
        /// Every export of the C API, so that the linker keeps them all in
        /// the binaries that link this crate (and `-rdynamic` exports them),
        /// and the tests can check each one's entry.
        pub static EXPORTS: &[Export] = &[
            $($(Export { name: stringify!($f), addr: $f as *const (), kind: $kind, gnu_allocates: $alloc },)*)*
        ];
    };
}

use errors::*;
use eval::*;
use memory::*;
use unwind::*;

unsafe extern "C" {
    // api.c
    fn TYPEOF();
    fn XLENGTH();
    fn LENGTH();
    fn Rf_XLENGTH();
    fn Rf_length();
    fn Rf_xlength();
    fn ATTRIB();
    fn OBJECT();
    fn LOGICAL();
    fn LOGICAL_RO();
    fn INTEGER();
    fn INTEGER_RO();
    fn REAL();
    fn REAL_RO();
    fn COMPLEX();
    fn COMPLEX_RO();
    fn RAW();
    fn RAW_RO();
    fn R_CHAR();
    fn STRING_ELT();
    fn VECTOR_ELT();
    fn SET_STRING_ELT();
    fn SET_VECTOR_ELT();
    fn CAR();
    fn CDR();
    fn TAG();
    fn CAAR();
    fn CDAR();
    fn CADR();
    fn CDDR();
    fn CDDDR();
    fn CADDR();
    fn CADDDR();
    fn CAD4R();
    fn CAD5R();
    fn SETCAR();
    fn SETCDR();
    fn SET_TAG();
    fn SETCADR();
    fn SETCADDR();
    fn PRINTNAME();
    fn R_ExternalPtrAddr();
    fn R_ExternalPtrTag();
    fn R_ExternalPtrProtected();
    fn R_ClearExternalPtr();
    fn R_SetExternalPtrAddr();
    fn R_SetExternalPtrTag();
    fn R_SetExternalPtrProtected();
    fn R_ExternalPtrAddrFn();
    fn R_WeakRefKey();
    fn R_WeakRefValue();
    fn Rf_protect();
    fn Rf_unprotect();
    fn R_ProtectWithIndex();
    fn R_Reprotect();
    fn Rf_unprotect_ptr();
    fn Rf_error();
    fn Rf_errorcall();
    fn Rf_warning();
    fn Rf_warningcall();
    fn Rprintf();
    fn REprintf();
    fn Rvprintf();
    fn REvprintf();
    fn R_UnwindProtect();
}

exports! {
    // the hot accessors, C over the layout (errors through entries)
    Kind::C, false =>
        TYPEOF XLENGTH LENGTH Rf_XLENGTH Rf_length Rf_xlength ATTRIB OBJECT LOGICAL LOGICAL_RO INTEGER
        INTEGER_RO REAL REAL_RO COMPLEX COMPLEX_RO RAW RAW_RO R_CHAR STRING_ELT VECTOR_ELT SET_STRING_ELT
        SET_VECTOR_ELT CAR CDR TAG CAAR CDAR CADR CDDR CDDDR CADDR CADDDR CAD4R CAD5R SETCAR SETCDR SET_TAG
        SETCADR SETCADDR PRINTNAME R_ExternalPtrAddr R_ExternalPtrTag R_ExternalPtrProtected R_ClearExternalPtr
        R_SetExternalPtrAddr R_SetExternalPtrTag R_SetExternalPtrProtected R_ExternalPtrAddrFn R_WeakRefKey
        R_WeakRefValue Rf_protect Rf_unprotect R_ProtectWithIndex R_Reprotect Rf_unprotect_ptr Rprintf REprintf
        Rvprintf REvprintf;
    // C over entries that allocate (the error's condition, the cleanup's
    // continuation)
    Kind::C, true => Rf_error Rf_errorcall Rf_warning Rf_warningcall R_UnwindProtect;
    // Rust leaves: no collection, no exit
    Kind::Leaf, false =>
        robigo_rtapi_store robigo_rtapi_print robigo_rtapi_warn_length R_ReleaseObject vmaxget vmaxset;
    // errors and warnings (the entries api.c calls)
    Kind::Entry(1), true =>
        robigo_rtapi_error robigo_rtapi_not_yet robigo_rtapi_warning robigo_rtapi_reprotect_error
        robigo_rtapi_charsxp_error;
    Kind::Entry(2), true => robigo_rtapi_errorcall robigo_rtapi_warningcall robigo_rtapi_extptr_error;
    Kind::Entry(3), true => robigo_rtapi_type_error robigo_rtapi_index_error robigo_rtapi_set_index_error;
    Kind::Entry(0), true => robigo_rtapi_protect_error robigo_rtapi_unprotect_error;
    // memory.c
    Kind::Entry(2), true => Rf_allocVector Rf_cons Rf_lcons Rf_list2 Rf_lang2 Rf_mkCharLen Rf_mkCharCE
        Rf_lengthgets Rf_xlengthgets R_RegisterCFinalizer R_RegisterFinalizer R_alloc;
    Kind::Entry(1), true => Rf_allocList Rf_list1 Rf_lang1 Rf_mkChar Rf_mkString Rf_ScalarInteger
        Rf_ScalarLogical Rf_ScalarString Rf_install Rf_duplicate Rf_shallow_duplicate
        Rf_PairToVectorList Rf_VectorToPairList R_PreserveObject;
    Kind::Entry(0), true => Rf_ScalarReal R_gc R_RunPendingFinalizers;
    Kind::Entry(3), true => Rf_list3 Rf_lang3 Rf_mkCharLenCE R_MakeExternalPtr R_RegisterCFinalizerEx
        R_RegisterFinalizerEx;
    Kind::Entry(4), true => Rf_list4 Rf_lang4 R_MakeWeakRef R_MakeWeakRefC;
    // eval.c, envir.c, attrib.c, coerce.c, util.c, source.c, options.c
    Kind::Entry(2), true => Rf_eval Rf_nthcdr Rf_getAttrib;
    Kind::Entry(3), true => R_tryEval R_NewEnv Rf_defineVar Rf_setAttrib;
    Kind::Entry(4), true => R_ParseVector;
    Kind::Entry(1), true => Rf_GetOption1 Rf_asInteger Rf_asLogical Rf_asReal robigo_rtapi_length_slow;
    // context.c
    Kind::Entry(2), true => R_ToplevelExec;
    Kind::Entry(4), true => robigo_rtapi_unwind_protect R_ExecWithCleanup;
    Kind::Entry(1), true => R_ContinueUnwind;
    Kind::Entry(0), true => R_MakeUnwindCont;
}

/// GNU R's numeric constants (`R_ext/Arith.h`): immutable process data,
/// the same in every isolate (L6 allows them: nothing writes them).
#[unsafe(no_mangle)]
pub static R_NaReal: f64 = robigo_heap::NA_REAL;
#[unsafe(no_mangle)]
pub static R_NaInt: i32 = robigo_heap::NA_INTEGER;
#[unsafe(no_mangle)]
pub static R_NaN: f64 = f64::NAN;
#[unsafe(no_mangle)]
pub static R_PosInf: f64 = f64::INFINITY;
#[unsafe(no_mangle)]
pub static R_NegInf: f64 = f64::NEG_INFINITY;

/// Make `cx`'s isolate one that runs package C code: `.Call` and
/// `dyn.load` bound (run after [`robigo_rt::install`], before
/// `robigo_rt::base::load`, which binds stubs only where nothing is
/// defined), the base DLL in the table, and the export table touched so
/// that every export is linked.
pub fn install(cx: &mut Cx) {
    std::hint::black_box(EXPORTS.as_ptr());
    std::hint::black_box([
        &R_NaReal as *const f64 as usize,
        &R_NaInt as *const i32 as usize,
        &R_NaN as *const f64 as usize,
        &R_PosInf as *const f64 as usize,
        &R_NegInf as *const f64 as usize,
    ]);
    dotcode::install(cx);
}

/// The layout `api.c` assumes (offsets and bits of robigo's objects), and
/// the heap's: equal, or `api.c` reads the wrong words.
pub fn check_layout() {
    unsafe extern "C" {
        static robigo_rtapi_layout: [usize; 11];
        static robigo_rtapi_layout_len: usize;
    }
    use robigo_heap::{OFF_DATA, OFF_LEN, extra, flags};
    let want = [
        1,
        6,
        8,
        OFF_LEN,
        OFF_DATA,
        16,
        24,
        32,
        flags::ALTREP as usize,
        flags::EXTDATA as usize,
        extra::SEEN_BY_C as usize,
    ];
    // SAFETY: constants of api.c.
    let (c, n) = unsafe { (robigo_rtapi_layout, robigo_rtapi_layout_len) };
    assert_eq!(n, want.len(), "robigo-rtapi: api.c's layout table has another length");
    assert_eq!(c, want, "robigo-rtapi: api.c's object layout is not robigo-heap's");
}
