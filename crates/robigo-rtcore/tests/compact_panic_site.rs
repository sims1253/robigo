//! In a debug build, the panic of a slice read of an unexpanded compact
//! sequence (`altrep::own`) names the site that read the slice, not the
//! accessor: the skew between #802 and #803 panicked deep in `as_slice`.
//! In a test binary of its own, since it installs a process-wide panic hook.
#![cfg(debug_assertions)]

use robigo_rtcore::{Cx, OwnedIsolate, Root, Scope};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;

/// `n1:n2` as `:` makes it.
fn range<'s>(cx: &mut Cx, s: Scope<'s>, n1: i64, n2: i64) -> Root<'s> {
    s.capture(cx, |cx, o| cx.compact_intrange(n1, n2, o)).unwrap()
}

/// The 1-based line of this file that holds `marker` (a comment that
/// survives reformatting; the search is written so it does not match itself).
fn line_of(marker: &str) -> u32 {
    let at = include_str!("compact_panic_site.rs").lines().position(|l| l.trim_end().ends_with(marker));
    at.expect("the marker is in this file") as u32 + 1
}

#[test]
fn a_debug_build_names_the_site_that_read_the_slice() {
    static AT: Mutex<Option<(String, u32)>> = Mutex::new(None);
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        *AT.lock().unwrap() = info.location().map(|l| (l.file().to_string(), l.line()));
    }));
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    let p = cx.scope(|cx, s| {
        let x = range(cx, s, 1, 3);
        let r = x.bind(cx);
        catch_unwind(AssertUnwindSafe(|| r.as_ints().map(|s| s.len()))) // the slice read
    });
    std::panic::set_hook(prev);
    assert!(p.is_err());
    let (file, at) = AT.lock().unwrap().clone().expect("a panic location");
    let want = line_of(concat!("// the slice", " read"));
    assert!(
        file.ends_with("compact_panic_site.rs") && at == want,
        "the panic was located at {file}:{at}, not line {want}"
    );
}
