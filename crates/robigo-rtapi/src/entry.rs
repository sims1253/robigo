//! Machine entries of the C API's exports (docs/g3.md §3 item 6, §6.5;
//! the foreman's decision on H2's question): every export that may collect
//! or exit is entered through a naked stub, generated per argument-count
//! class by [`entry!`], that pushes the callee-saved registers C left in
//! place just below its return address and calls the export's Rust body
//! with the lowest of their addresses as one more argument. The body closes
//! the C segment there ([`robigo_rtcore::enter_export`]): the segment holds
//! C's frames and registers and no word of a Rust frame, which is never
//! scanned (§3).
//!
//! The class is the number of integer-class arguments (x86-64 SysV: rdi,
//! rsi, rdx, rcx, r8, r9; a `double` goes in an SSE register and does not
//! count): the stub hands `lo` over in the next integer register, so the
//! body is the export's own signature plus `lo: usize`. Classes 0 to 5
//! exist; an export of six integer arguments would pass `lo` on the stack,
//! and none needs it. A constant assertion checks each export's class
//! against its argument types ([`CArg::INT`]).
//!
//! The body's arguments are rooted for its duration ([`CArg::enter`]: a
//! `SEXP` becomes a [`Root`] of the export's scope, marked as seen by C
//! under `ROBIGO_GC_MOVE=1`), and its result is handed back to C
//! ([`CRet::to_c`]). An exit leaves through [`robigo_rtcore::Landing::jump`]
//! once the body's frames hold nothing to drop.
//!
//! Exports that cannot collect, exit or run R code (accessors, `Rprintf`)
//! are plain `extern "C"` functions or C (`api.c`), with no stub.
//!
//! Elsewhere than x86-64 the registers are spilled into the segment node
//! instead and the node's address is `lo` (the heap's documented fallback:
//! the body's frame above the node is scanned too).

use robigo_rtcore::{Cx, RResult, Ref, Root, Scope, ScanSegment, Sexp};
use std::ffi::{c_char, c_int, c_uint, c_void};

/// One argument of an export, as its body receives it.
pub trait CArg<'s>: Copy {
    /// Passed in an integer register (x86-64 SysV's INTEGER class).
    const INT: bool;
    /// What the body sees.
    type Body;
    /// Make the body's view: a `SEXP` is rooted in `s` for the body's
    /// duration (docs/g3.md §3 item 6) and marked as handed to C.
    fn enter(self, cx: &mut Cx, s: Scope<'s>) -> Self::Body;
}

impl<'s> CArg<'s> for Sexp {
    const INT: bool = true;
    type Body = Root<'s>;
    #[inline(always)]
    fn enter(self, cx: &mut Cx, s: Scope<'s>) -> Root<'s> {
        cx.hand_to_c(self);
        // SAFETY: C's contract for every SEXP argument of the API: a live
        // object (of this isolate: rooting checks the owner and panics
        // otherwise, which aborts in an export).
        s.root(cx, unsafe { Ref::from_sexp(self, cx) })
    }
}

/// A `SEXP` argument C may pass as NULL (`R_tryEval`'s `env`): rooted
/// when it is an object.
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct OrNull(pub Sexp);

impl<'s> CArg<'s> for OrNull {
    const INT: bool = true;
    type Body = Option<Root<'s>>;
    #[inline(always)]
    fn enter(self, cx: &mut Cx, s: Scope<'s>) -> Option<Root<'s>> {
        (!self.0.is_null()).then(|| self.0.enter(cx, s))
    }
}

macro_rules! plain_args {
    ($int:expr => $($t:ty),*) => {
        $(impl<'s> CArg<'s> for $t {
            const INT: bool = $int;
            type Body = $t;
            #[inline(always)]
            fn enter(self, _: &mut Cx, _: Scope<'s>) -> $t {
                self
            }
        })*
    };
}
plain_args!(true => c_int, c_uint, isize, usize, *const c_char, *mut c_void, *const c_void, *mut c_int, *mut Sexp);
plain_args!(false => f64);

impl<'s, A, B> CArg<'s> for unsafe extern "C" fn(A) -> B {
    const INT: bool = true;
    type Body = Self;
    #[inline(always)]
    fn enter(self, _: &mut Cx, _: Scope<'s>) -> Self {
        self
    }
}

impl<'s, A, B, C> CArg<'s> for unsafe extern "C" fn(A, B) -> C {
    const INT: bool = true;
    type Body = Self;
    #[inline(always)]
    fn enter(self, _: &mut Cx, _: Scope<'s>) -> Self {
        self
    }
}

/// What an export's body returns, as C receives it.
pub trait CRet {
    type C;
    fn to_c(self, cx: &Cx) -> Self::C;
}

impl CRet for Sexp {
    type C = Sexp;
    /// A result object is C's from here (held by a C local, pinned by the
    /// conservative scan, protected by C if it lives on): marked as seen by
    /// C under `ROBIGO_GC_MOVE=1`.
    #[inline(always)]
    fn to_c(self, cx: &Cx) -> Sexp {
        cx.hand_to_c(self);
        self
    }
}

macro_rules! plain_ret {
    ($($t:ty),*) => {
        $(impl CRet for $t {
            type C = $t;
            #[inline(always)]
            fn to_c(self, _: &Cx) -> $t {
                self
            }
        })*
    };
}
plain_ret!((), c_int, c_uint, isize, usize, f64, *mut c_void, *const c_char);

/// The body of every machine entry: close the segment at `lo`, run `f` in a
/// root scope with a capability of the thread's isolate, reopen, and return
/// `f`'s result to C, or jump with its exit.
///
/// # Safety
/// Called only by an [`entry!`] body, with the `lo` its stub passed.
#[inline(always)]
pub unsafe fn run<T: CRet>(lo: usize, seg: &mut ScanSegment, f: impl FnOnce(&mut Cx) -> RResult<T>) -> T::C {
    // SAFETY: the stub's contract: `lo` is the lowest register it pushed,
    // and `seg` lives in this frame, below `lo`, until this returns.
    let r = unsafe { robigo_rtcore::enter_export(lo, seg, |cx| f(cx).map(|v| v.to_c(cx))) };
    match r {
        Ok(v) => v,
        // SAFETY: the capability ended with enter_export, which reopened the
        // segment; this frame and the body's hold nothing to drop.
        Err(l) => unsafe { l.jump() },
    }
}

/// The x86-64 stub of class `$n`: the register that receives `lo`.
#[cfg(target_arch = "x86_64")]
#[macro_export]
#[doc(hidden)]
macro_rules! __entry_lo_reg {
    (0) => {
        "rdi"
    };
    (1) => {
        "rsi"
    };
    (2) => {
        "rdx"
    };
    (3) => {
        "rcx"
    };
    (4) => {
        "r8"
    };
    (5) => {
        "r9"
    };
}

/// Define C API exports with machine entries:
///
/// ```ignore
/// entry! {
///     /// GNU R's `Rf_cons(car, cdr)`
///     [2] fn Rf_cons(car: Sexp, cdr: Sexp) -> Sexp => |cx, s, car, cdr| { ... }
/// }
/// ```
///
/// `[n]` is the argument-count class (integer-class arguments); the body
/// gets the capability, the export's root scope and the arguments as
/// [`CArg::enter`] made them, and returns `RResult` of the C result.
#[macro_export]
macro_rules! entry {
    ($(
        $(#[$m:meta])*
        [$n:tt] fn $name:ident($($a:ident: $t:ty),* $(,)?) -> $ret:ty => |$cx:ident, $s:ident| $body:block
    )*) => {$(
        $crate::__entry_one! { $(#[$m])* [$n] fn $name($($a: $t),*) -> $ret => |$cx, $s| $body }
    )*};
}

#[cfg(target_arch = "x86_64")]
#[macro_export]
#[doc(hidden)]
macro_rules! __entry_one {
    ($(#[$m:meta])* [$n:tt] fn $name:ident($($a:ident: $t:ty),*) -> $ret:ty => |$cx:ident, $s:ident| $body:block) => {
        const _: () = assert!(
            0 $(+ (<$t as $crate::entry::CArg<'static>>::INT as usize))* == $n,
            concat!("robigo-rtapi: ", stringify!($name), "'s argument-count class is not ", stringify!($n))
        );
        $(#[$m])*
        #[unsafe(no_mangle)]
        #[unsafe(naked)]
        pub extern "C" fn $name($($a: $t),*) -> $ret {
            // push the callee-saved registers C left in place, hand the
            // lowest one's address to the body, keep the call 16-byte
            // aligned; the epilogue leaves rax/rdx/xmm0 as the body set them
            ::core::arch::naked_asm!(
                "push rbp",
                "push rbx",
                "push r12",
                "push r13",
                "push r14",
                "push r15",
                concat!("mov ", $crate::__entry_lo_reg!($n), ", rsp"),
                "sub rsp, 8",
                "call {body}",
                "add rsp, 8",
                "pop r15",
                "pop r14",
                "pop r13",
                "pop r12",
                "pop rbx",
                "pop rbp",
                "ret",
                body = sym $name::body,
            );
        }
        // the body, in a module of the export's name (a naked function holds
        // its assembly only)
        #[allow(non_snake_case)]
        #[doc(hidden)]
        mod $name {
            #[allow(unused_imports)]
            use super::*;
            pub(super) extern "C" fn body($($a: $t,)* lo: usize) -> $ret {
                let mut seg = $crate::entry::ScanSegmentNode::new();
                let f = |$cx: &mut ::robigo_rtcore::Cx| {
                    $cx.scope(|$cx, $s| -> ::robigo_rtcore::RResult<$ret> {
                        $(let $a = $crate::entry::CArg::enter($a, $cx, $s);)*
                        $body
                    })
                };
                // SAFETY: called by this export's stub, which passes `lo`.
                unsafe { $crate::entry::run(lo, seg.get(), f) }
            }
        }
    };
}

#[cfg(not(target_arch = "x86_64"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __entry_one {
    ($(#[$m:meta])* [$n:tt] fn $name:ident($($a:ident: $t:ty),*) -> $ret:ty => |$cx:ident, $s:ident| $body:block) => {
        $(#[$m])*
        #[unsafe(no_mangle)]
        pub extern "C" fn $name($($a: $t),*) -> $ret {
            let mut seg = $crate::entry::ScanSegmentNode::new();
            let lo = seg.spill();
            let f = |$cx: &mut ::robigo_rtcore::Cx| {
                $cx.scope(|$cx, $s| -> ::robigo_rtcore::RResult<$ret> {
                    $(let $a = $crate::entry::CArg::enter($a, $cx, $s);)*
                    $body
                })
            };
            // SAFETY: the registers are in the node, whose address is `lo`.
            unsafe { $crate::entry::run(lo, seg.get(), f) }
        }
    };
}

/// The segment node of one export call, in the body's frame.
#[doc(hidden)]
pub struct ScanSegmentNode(ScanSegment);

impl ScanSegmentNode {
    #[inline(always)]
    pub fn new() -> Self {
        ScanSegmentNode(ScanSegment::new())
    }

    #[inline(always)]
    pub fn get(&mut self) -> &mut ScanSegment {
        &mut self.0
    }

    /// Spill the callee-saved registers into the node; its address is the
    /// segment's low end (no machine entry: not x86-64).
    #[inline(always)]
    pub fn spill(&mut self) -> usize {
        self.0.spill_registers();
        &self.0 as *const ScanSegment as usize
    }
}

impl Default for ScanSegmentNode {
    fn default() -> Self {
        Self::new()
    }
}
