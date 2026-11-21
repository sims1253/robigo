//! memory.c's `.Internal(gc(verbose, reset, full))` and
//! `.Internal(reg.finalizer(e, f, onexit))`, over the heap's collection and
//! weak references ([`crate::fin`]).
//!
//! `gc()`'s numbers, `gcinfo`'s reports and `memory.profile()`'s counts are
//! the robigo heap's, in GNU R's units and shape: nodes counted as
//! `sizeof(SEXPREC)` = 56 bytes, the vector heap in 8-byte VECRECs, every
//! collection a full one (GNU R's level 2). docs/g3.md's L7 asks for GNU
//! R's own numbers, which need a logical account of GNU R's allocations
//! that does not exist yet (docs/ledger/g3/2026-11-18-h-memory-internals.md).

use robigo_heap::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, Ref, Root};

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
    crate::prims::check_arity(cx, call, args.len(), "gc", 3, true)?;
    let verbose = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 0));
    let reset_max = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 1));
    let _full = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 2));
    // gc_reporting = asLogical(verbose) for this collection: NA reports too
    let ogc = cx.set_gc_reporting(verbose != 0);
    crate::fin::gc(cx);
    cx.set_gc_reporting(ogc);
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
pub fn do_reg_finalizer(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "reg.finalizer", 3, true)?;
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

/// `.Internal(mem.maxVSize(vsize))`: a positive `vsize` (MiB) becomes
/// the vector memory limit, `Inf` lifts it, one below the current usage is
/// refused with a warning; the value is the limit in MiB, `Inf` when there
/// is none.
// R: memory.c:do_maxVSize
pub fn do_max_vsize(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    const MB: f64 = 1048576.0;
    // asReal(CAR(args)), with no checkArity: other arguments are ignored,
    // none is R_NilValue's
    let newval = cx.scope(|cx, s| {
        let x = if args.is_empty() { s.root(cx, cx.nil()) } else { args.root(0) };
        robigo_rtbuiltins::coerce::vector::as_real_warn(cx, x)
    })?;
    if newval > 0.0 {
        if newval == f64::INFINITY {
            cx.set_max_vsize(None);
        } else {
            let newbytes = newval * MB;
            // (R_SIZE_T_MAX: no limit)
            if newbytes >= usize::MAX as f64 {
                cx.set_max_vsize(None);
            } else if !cx.set_max_vsize(Some(newbytes as usize)) {
                cx.warning("a limit lower than current usage, so ignored")?;
            }
        }
    }
    let v = cx.max_vsize().map_or(f64::INFINITY, |b| b as f64 / MB);
    cx.build_scalar(out, v)
}

/// `.Internal(gctorture(on))`: the previous state, and a collection due at
/// every allocation from now on (`TRUE`), none (`FALSE`), or every `on`th
/// (a number; NA or negative changes nothing). A logical `on` is
/// `asRbool`'s: NA, or more than one element, is an error. Each is served at
/// the next safepoint.
// R: memory.c:do_gctorture
pub fn do_gctorture(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "gctorture", 1, true)?;
    let old = cx.gc_torture_on();
    let gap = if args.get(cx, 0).sexptype() == LGLSXP {
        // asRbool(CAR(args), call), which never returns NA_LOGICAL
        robigo_rtbuiltins::util::as_bool2(cx, call, args.root(0))? as i32
    } else {
        robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(0))?
    };
    // R_gc_torture(gap, 0, FALSE): NA (negative) changes nothing
    cx.gc_torture(gap, 0);
    out.set(cx, cx.logical_value(old as i32));
    Ok(())
}

/// `.Internal(gcinfo(verbose))`: whether collections report from now on
/// (NA changes nothing); the previous state.
// R: memory.c:do_gcinfo
pub fn do_gcinfo(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "gcinfo", 1, true)?;
    let old = cx.gc_reporting();
    let i = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 0));
    if i != i32::MIN {
        cx.set_gc_reporting(i != 0);
    }
    out.set(cx, cx.logical_value(old as i32));
    Ok(())
}

/// `gc.time(on = TRUE)` (a primitive): time collections from now on, or
/// stop; the times spent in collections while it was on: user, system,
/// elapsed, child user, child system.
// R: memory.c:do_gctime
pub fn do_gctime(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let on = if args.is_empty() {
        true
    } else {
        robigo_rtbuiltins::util::check1arg(cx, args, call, "on")?;
        robigo_rtbuiltins::util::as_bool2(cx, call, args.root(0))?
    };
    let t = cx.gc_time(on);
    cx.build(out, |cx| Ok(cx.alloc_copy(&t)?.finish().into()))
}

/// `.Internal(gctorture2(step, wait, inhibit_release))`: torture every
/// `step` allocations after `wait` of them; the previous step. GNU R reads
/// `inhibit_release` only in a PROTECTCHECK build, but checks it always.
// R: memory.c:do_gctorture2
pub fn do_gctorture2(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "gctorture2", 3, true)?;
    let old = cx.gc_torture_gap();
    let gap = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(0))?;
    let wait = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(1))?;
    let _inhibit = robigo_rtbuiltins::util::as_bool2(cx, call, args.root(2))?;
    cx.gc_torture(gap, wait);
    cx.build_scalar(out, old)
}

/// `.Internal(memory.profile())`: after a full collection, the heap's
/// objects counted by type, named by `type2str` (types 11 and 12 skipped).
// R: memory.c:do_memoryprofile
pub fn do_memoryprofile(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "memory.profile", 0, true)?;
    // R_gc() (finalizers run after it, as R_gc's)
    crate::fin::gc(cx);
    let by = cx.census_by_type();
    let ty = |i: usize| if i > LGLSXP as usize { i + 2 } else { i };
    cx.build(out, |cx| {
        let mut nms = cx.alloc_vector::<Char<'_>>(24)?;
        for i in 0..24 {
            nms.set(i, cx.mk_char(robigo_rtbuiltins::util::type2char(ty(i) as u8).as_bytes(), Encoding::Native)?);
        }
        let nms = nms.finish();
        let counts: Vec<i32> = (0..24).map(|i| by[ty(i)] as i32).collect();
        let mut ans = cx.alloc_copy(&counts)?;
        let attrs = cx.cons_tagged(nms.as_ref(), cx.nil(), cx.sym(robigo_rtcore::RSym::Names))?;
        ans.set_attrib(attrs);
        Ok(Ref::from(ans.finish()))
    })
}

/// `.Internal(mem.maxNSize(nsize))`: as `mem.maxVSize`, for the node
/// limit, counted in nodes.
// R: memory.c:do_maxNSize
pub fn do_max_nsize(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let newval = cx.scope(|cx, s| {
        let x = if args.is_empty() { s.root(cx, cx.nil()) } else { args.root(0) };
        robigo_rtbuiltins::coerce::vector::as_real_warn(cx, x)
    })?;
    if newval > 0.0 {
        if newval == f64::INFINITY || newval >= usize::MAX as f64 {
            cx.set_max_nsize(None);
        } else if !cx.set_max_nsize(Some(newval as usize)) {
            cx.warning("a limit lower than current usage, so ignored")?;
        }
    }
    let v = cx.max_nsize().map_or(f64::INFINITY, |n| n as f64);
    cx.build_scalar(out, v)
}
