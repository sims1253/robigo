//! times.c: `currentTime()` (the time now, in seconds since the epoch, as
//! `timespec_get(&tp, TIME_UTC)` gives it) and `.Internal(Sys.time())`
//! (`do_systime`).

use robigo_rtcore::{Args, Cx, Out, RResult, Root};

/// `currentTime()`: `(double) tp.tv_sec + 1e-9 * (double) tp.tv_nsec` of
/// the wall clock (`timespec_get(..., TIME_UTC)`).
// R: times.c:currentTime
pub fn current_time() -> f64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as f64 + 1e-9 * d.subsec_nanos() as f64,
        // a clock before 1970: timespec's tv_sec is negative and tv_nsec in
        // [0, 1e9)
        Err(e) => {
            let d = e.duration();
            if d.subsec_nanos() == 0 {
                -(d.as_secs() as f64)
            } else {
                -(d.as_secs() as f64 + 1.0) + 1e-9 * (1_000_000_000 - d.subsec_nanos()) as f64
            }
        }
    }
}

/// `.Internal(Sys.time())`: [`current_time`].
// R: times.c:do_systime
pub fn do_systime(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("Sys.time", true).check_arity(cx, call, args)?;
    cx.build_scalar(out, current_time())
}
