// A capability cannot be moved to another owner (Sol's R1a review, S1):
// `Cx` is unsized, so neither swap compiles. The first moved a callback's
// capability into another owner; the second left `ca` naming B's isolate
// after `b` was dropped. `exchange` has `std::mem::swap`'s signature (and
// calls it): its implicit `Sized` bound is what refuses a `Cx`, and naming it
// here keeps this record free of std's source (which CI's toolchain lacks).
use robigo_rtcore::OwnedIsolate;

fn exchange<T>(a: &mut T, b: &mut T) {
    std::mem::swap(a, b)
}

fn main() {
    let mut a = OwnedIsolate::new();
    let mut b = OwnedIsolate::new();
    a.cx().call_out(|re| re.enter(|callback| exchange(callback, b.cx())));

    let ca = a.cx();
    exchange(ca, b.cx());
    drop(b);
    let _ = ca.nil();
}
