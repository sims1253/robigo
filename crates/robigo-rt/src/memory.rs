//! memory.c's `.Internal(gc(verbose, reset, full))` and
//! `.Internal(reg.finalizer(e, f, onexit))`, over the heap's collection and
//! weak references ([`crate::fin`]).
//!
//! `gc()`'s numbers are the robigo heap's, in GNU R's units: nodes counted
//! as `sizeof(SEXPREC)` = 56 bytes, the vector heap in 8-byte VECRECs. The
//! verbose report (`gcinfo`) is not printed: `verbose = TRUE` says so.

use robigo_heap::types::*;
use robigo_rtcore::{Args, Cx, Out, RResult, Root};

/// `sizeof(SEXPREC)` on a 64-bit build, and `Mega`.
const SEXPREC: f64 = 56.0;
const VSFAC: f64 = 8.0;
const MEGA: f64 = 1048576.0;

/// `0.1 * ceil(10 * x)`.
fn mb(x: f64) -> f64 {
    0.1 * (10.0 * x).ceil()
}

/// `.Internal(gc(verbose, reset, full))`: collect (a full collection for
/// `full`; robigo's collections are all full), run pending finalizers,
/// then the 14 numbers `gc()` shapes into its table.
// R: memory.c:do_gc
pub fn do_gc(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if args.len() != 3 {
        let msg = format!(
            "{} argument{} passed to .Internal(gc) which requires 3",
            args.len(),
            if args.len() == 1 { "" } else { "s" }
        );
        return Err(cx.error(&msg));
    }
    let verbose = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 0));
    let reset_max = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 1));
    let _full = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 2));
    if verbose != 0 && verbose != i32::MIN {
        return Err(robigo_rtbuiltins::util::not_yet(cx, Some(call), "gc(verbose = TRUE) (gcinfo's report)"));
    }
    crate::fin::gc(cx);
    if reset_max != 0 && reset_max != i32::MIN {
        cx.reset_maxused();
    }
    let h = cx.heap_report();
    let ncells = h.nodes_in_use as f64;
    let vcells = h.vecrecs_in_use as f64;
    let v = [
        ncells,
        vcells,
        mb(ncells / MEGA * SEXPREC),
        mb(vcells / MEGA * VSFAC),
        h.n_size as f64,
        h.v_size as f64,
        mb(h.n_size as f64 / MEGA * SEXPREC),
        mb(h.v_size as f64 / MEGA * VSFAC),
        h.max_n_size.map_or(robigo_text::NA_REAL, |n| mb(n as f64 / MEGA * SEXPREC)),
        h.max_v_size.map_or(robigo_text::NA_REAL, |b| mb(b as f64 / MEGA)),
        h.n_maxused as f64,
        h.v_maxused as f64,
        mb(h.n_maxused as f64 / MEGA * SEXPREC),
        mb(h.v_maxused as f64 / MEGA * VSFAC),
    ];
    cx.build(out, |cx| Ok(cx.alloc_copy(&v)?.finish().into()))
}

/// `.Internal(reg.finalizer(e, f, onexit))`.
// R: memory.c:do_regFinaliz
pub fn do_reg_finalizer(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if args.len() != 3 {
        let msg = format!(
            "{} argument{} passed to .Internal(reg.finalizer) which requires 3",
            args.len(),
            if args.len() == 1 { "" } else { "s" }
        );
        return Err(cx.error(&msg));
    }
    if !matches!(args.get(cx, 0).sexptype(), ENVSXP | EXTPTRSXP) {
        return Err(cx.error("first argument must be environment or external pointer"));
    }
    if args.get(cx, 1).sexptype() != CLOSXP {
        return Err(cx.error("second argument must be a function"));
    }
    let onexit = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 2));
    if onexit == i32::MIN {
        return Err(cx.error("third argument must be 'TRUE' or 'FALSE'"));
    }
    crate::fin::register_finalizer(cx, args.root(0), args.root(1), onexit != 0)?;
    out.set(cx, cx.nil());
    Ok(())
}
