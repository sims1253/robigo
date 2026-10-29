//! The process-wide state behind the API: one heap, the API's global
//! `SEXP`s (exported data symbols), the protect stack in GNU R's layout.

use crate::host::Host;
use robigo_core::types::*;
use robigo_core::{Core, Sexp};
use std::ffi::c_int;

pub struct State {
    pub core: Box<Core>,
    pub host: Option<Host>,
    /// R_alloc's stack: RAWSXPs chained through their attribute slot
    pub vstack: Sexp,
}

// The isolate record (lead requirement A.2, docs/core.md §10): one per
// OS thread, reached through a thread-local pointer, never a process-wide
// singleton. The exported data symbols below (R_NilValue, R_GlobalEnv,
// R_PPStack, ...) are still process-wide variables; robigo's headers
// turn the per-isolate ones into macros over this record.
thread_local! {
    static ISOLATE: std::cell::Cell<*mut State> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// This thread's isolate, or null.
#[inline(always)]
pub fn isolate() -> *mut State {
    ISOLATE.with(|c| c.get())
}

#[inline(always)]
pub fn st() -> &'static mut State {
    unsafe {
        let p = isolate();
        if p.is_null() {
            init_state();
            return &mut *isolate();
        }
        &mut *p
    }
}

#[inline(always)]
pub fn core() -> &'static mut Core {
    &mut st().core
}

#[inline(always)]
pub fn nil() -> Sexp {
    unsafe { R_NilValue }
}

// ---- exported data (Rinternals.h, R_ext/*.h) -------------------------------

macro_rules! sexp_globals {
    ($($name:ident),* $(,)?) => {
        $(#[unsafe(no_mangle)] pub static mut $name: Sexp = Sexp::NULL;)*
    };
}

sexp_globals!(
    R_NilValue,
    R_UnboundValue,
    R_MissingArg,
    R_GlobalEnv,
    R_EmptyEnv,
    R_BaseEnv,
    R_BaseNamespace,
    R_NamespaceRegistry,
    R_Srcref,
    R_InBCInterpreter,
    R_CurrentExpression,
    R_RestartToken,
    R_NaString,
    R_BlankString,
    R_BlankScalarString,
    R_TrueValue,
    R_FalseValue,
    R_LogicalNAValue,
    R_InterruptToken,
);

/// Symbols the API exports as data, with their names (names.c).
pub static SYMBOLS: &[(&str, &str)] = &[
    ("R_Bracket2Symbol", "[["),
    ("R_BracketSymbol", "["),
    ("R_BraceSymbol", "{"),
    ("R_DoubleColonSymbol", "::"),
    ("R_ClassSymbol", "class"),
    ("R_DeviceSymbol", ".Device"),
    ("R_DevicesSymbol", ".Devices"),
    ("R_DimNamesSymbol", "dimnames"),
    ("R_DimSymbol", "dim"),
    ("R_DollarSymbol", "$"),
    ("R_DotsSymbol", "..."),
    ("R_DropSymbol", "drop"),
    ("R_EvalSymbol", "eval"),
    ("R_FunctionSymbol", "function"),
    ("R_LastvalueSymbol", ".Last.value"),
    ("R_LevelsSymbol", "levels"),
    ("R_ModeSymbol", "mode"),
    ("R_NameSymbol", "name"),
    ("R_NamesSymbol", "names"),
    ("R_NaRmSymbol", "na.rm"),
    ("R_PackageSymbol", "package"),
    ("R_PreviousSymbol", "previous"),
    ("R_QuoteSymbol", "quote"),
    ("R_RowNamesSymbol", "row.names"),
    ("R_SeedsSymbol", ".Random.seed"),
    ("R_SortListSymbol", "sort.list"),
    ("R_SourceSymbol", "source"),
    ("R_SpecSymbol", "spec"),
    ("R_TripleColonSymbol", ":::"),
    ("R_TspSymbol", "tsp"),
    ("R_dot_defined", ".defined"),
    ("R_dot_Method", ".Method"),
    ("R_dot_packageName", ".packageName"),
    ("R_dot_target", ".target"),
    ("R_dot_Generic", ".Generic"),
    ("R_SrcrefSymbol", "srcref"),
    ("R_SrcfileSymbol", "srcfile"),
    ("R_WholeSrcrefSymbol", "wholeSrcref"),
    ("R_TmpvalSymbol", "*tmp*"),
    ("R_UseNamesSymbol", "use.names"),
    ("R_ColonSymbol", ":"),
    ("R_ExactSymbol", "exact"),
    ("R_RecursiveSymbol", "recursive"),
    ("R_AsCharacterSymbol", "as.character"),
    ("R_baseSymbol", "base"),
    ("R_BaseSymbol", "base"),
    ("R_dot_Generic", ".Generic"),
    ("R_dot_Methods", ".Methods"),
    ("R_dot_Group", ".Group"),
    ("R_dot_Class", ".Class"),
    ("R_dot_GenericCallEnv", ".GenericCallEnv"),
    ("R_dot_GenericDefEnv", ".GenericDefEnv"),
];

macro_rules! sym_globals {
    ($($name:ident),* $(,)?) => {
        $(#[unsafe(no_mangle)] pub static mut $name: Sexp = Sexp::NULL;)*
        unsafe fn set_symbol_global(name: &str, v: Sexp) {
            match name {
                $(stringify!($name) => $name = v,)*
                _ => {}
            }
        }
    };
}

sym_globals!(
    R_Bracket2Symbol,
    R_BracketSymbol,
    R_BraceSymbol,
    R_DoubleColonSymbol,
    R_ClassSymbol,
    R_DeviceSymbol,
    R_DevicesSymbol,
    R_DimNamesSymbol,
    R_DimSymbol,
    R_DollarSymbol,
    R_DotsSymbol,
    R_DropSymbol,
    R_EvalSymbol,
    R_FunctionSymbol,
    R_LastvalueSymbol,
    R_LevelsSymbol,
    R_ModeSymbol,
    R_NameSymbol,
    R_NamesSymbol,
    R_NaRmSymbol,
    R_PackageSymbol,
    R_PreviousSymbol,
    R_QuoteSymbol,
    R_RowNamesSymbol,
    R_SeedsSymbol,
    R_SortListSymbol,
    R_SourceSymbol,
    R_SpecSymbol,
    R_TripleColonSymbol,
    R_TspSymbol,
    R_dot_defined,
    R_dot_Method,
    R_dot_packageName,
    R_dot_target,
    R_dot_Generic,
    R_SrcrefSymbol,
    R_SrcfileSymbol,
    R_WholeSrcrefSymbol,
    R_TmpvalSymbol,
    R_UseNamesSymbol,
    R_ColonSymbol,
    R_ExactSymbol,
    R_RecursiveSymbol,
    R_AsCharacterSymbol,
    R_baseSymbol,
    R_BaseSymbol,
    R_dot_Methods,
    R_dot_Group,
    R_dot_Class,
    R_dot_GenericCallEnv,
    R_dot_GenericDefEnv,
);

/// GNU R's default protect stack size (R_PPSSIZE).
pub const PPSSIZE: c_int = 50000;

#[unsafe(no_mangle)]
pub static mut R_PPStack: *mut Sexp = std::ptr::null_mut();
#[unsafe(no_mangle)]
pub static mut R_PPStackTop: c_int = 0;
#[unsafe(no_mangle)]
pub static mut R_PPStackSize: c_int = 0;

// Flags and settings packages read directly.
#[unsafe(no_mangle)]
pub static mut R_interrupts_pending: c_int = 0;
#[unsafe(no_mangle)]
pub static mut R_interrupts_suspended: c_int = 0;
#[unsafe(no_mangle)]
pub static mut R_Interactive: c_int = 0;
#[unsafe(no_mangle)]
pub static mut R_isForkedChild: c_int = 0;
#[unsafe(no_mangle)]
pub static mut R_CStackLimit: usize = usize::MAX;
#[unsafe(no_mangle)]
pub static mut R_CStackStart: usize = usize::MAX;
#[unsafe(no_mangle)]
pub static mut R_CStackDir: c_int = 1;
#[unsafe(no_mangle)]
pub static mut R_Visible: c_int = 1;
#[unsafe(no_mangle)]
pub static mut R_NoEcho: c_int = 0;
#[unsafe(no_mangle)]
pub static mut mbcslocale: c_int = 1;
#[unsafe(no_mangle)]
pub static mut utf8locale: c_int = 1;
#[unsafe(no_mangle)]
pub static mut latin1locale: c_int = 0;
#[unsafe(no_mangle)]
pub static mut known_to_be_utf8: c_int = 1;
#[unsafe(no_mangle)]
pub static mut known_to_be_latin1: c_int = 0;
#[unsafe(no_mangle)]
pub static mut R_dec_min_exponent: c_int = -308;
#[unsafe(no_mangle)]
pub static mut R_num_math_threads: c_int = 1;
#[unsafe(no_mangle)]
pub static mut R_max_num_math_threads: c_int = 1;
#[unsafe(no_mangle)]
pub static mut R_Is_Running: c_int = 2;
#[unsafe(no_mangle)]
pub static mut R_TempDir: *mut std::ffi::c_char = std::ptr::null_mut();
#[unsafe(no_mangle)]
pub static mut R_Home: *mut std::ffi::c_char = std::ptr::null_mut();

unsafe fn mark_api_roots(c: &mut Core) {
    for i in 0..R_PPStackTop.max(0) as usize {
        c.mark(*R_PPStack.add(i));
    }
    let s = &*isolate();
    c.mark(s.vstack);
    for v in [
        R_GlobalEnv,
        R_EmptyEnv,
        R_BaseEnv,
        R_BaseNamespace,
        R_NamespaceRegistry,
        R_Srcref,
        R_CurrentExpression,
        R_RestartToken,
        R_BlankScalarString,
        R_TrueValue,
        R_FalseValue,
        R_LogicalNAValue,
        R_InterruptToken,
    ] {
        if !v.is_null() {
            c.mark(v);
        }
    }
    crate::errors::mark_roots(c);
    crate::rdynload::mark_roots(c);
    crate::graphics::mark_roots(c);
    crate::bridge::mark_proxies(c);
}

/// Create the heap and the API's constants (memory.c InitMemory,
/// names.c InitNames). Idempotent.
pub unsafe fn init_state() {
    if !isolate().is_null() {
        return;
    }
    let mut heap = Core::new();
    R_NilValue = heap.nil;
    R_UnboundValue = heap.unbound;
    R_MissingArg = heap.missing_arg;
    R_NaString = heap.na_string;
    R_BlankString = heap.blank_string;
    heap.root_fns.push(mark_api_roots);
    let nil = heap.nil;
    let state = Box::into_raw(Box::new(State { core: heap, host: None, vstack: nil }));
    ISOLATE.with(|c| c.set(state));
    let stack = vec![Sexp::NULL; PPSSIZE as usize + 1000].into_boxed_slice();
    R_PPStack = Box::into_raw(stack) as *mut Sexp;
    R_PPStackSize = PPSSIZE;
    R_PPStackTop = 0;
    let c = core();
    c.gc_inhibit += 1;
    for &(g, name) in SYMBOLS {
        let s = c.install(name.as_bytes());
        set_symbol_global(g, s);
    }
    let lgl = |c: &mut Core, v: i32| {
        let x = c.alloc_vector(LGLSXP, 1);
        *(x.std_data() as *mut i32) = v;
        c.make_immortal(x);
        x
    };
    R_TrueValue = lgl(c, 1);
    R_FalseValue = lgl(c, 0);
    R_LogicalNAValue = lgl(c, robigo_core::NA_LOGICAL);
    let bs = c.alloc_vector(STRSXP, 1);
    c.make_immortal(bs);
    R_BlankScalarString = bs;
    R_RestartToken = c.alloc_node(LISTSXP, nil, nil, nil);
    c.make_immortal(R_RestartToken);
    R_InterruptToken = c.alloc_node(LISTSXP, nil, nil, nil);
    c.make_immortal(R_InterruptToken);
    R_InBCInterpreter = c.alloc_node(LISTSXP, nil, nil, nil);
    c.make_immortal(R_InBCInterpreter);
    R_CurrentExpression = c.alloc_node(LISTSXP, nil, nil, nil);
    c.make_immortal(R_CurrentExpression);
    R_Srcref = nil;
    c.gc_inhibit -= 1;
    crate::arith::init_arith();
    // the core is bound to this thread (the evaluator's): GNU R's C stack
    // check uses its bounds, the stack grows down (R_CStackDir 1), and
    // the limit is 95% of the stack as setup_Rmainloop sets it
    let (lo, top) = c.stack_bounds();
    if top != 0 {
        R_CStackStart = top;
        R_CStackDir = 1;
        R_CStackLimit = ((top - lo) as f64 * 0.95) as usize;
    }
}
