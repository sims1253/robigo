// A capability cannot be moved to another owner (Sol's R1a review, S1):
// `Cx` is unsized, so neither swap compiles. The first moved a callback's
// capability into another owner; the second left `ca` naming B's isolate
// after `b` was dropped.
use robigo_rtcore::OwnedIsolate;

fn main() {
    let mut a = OwnedIsolate::new();
    let mut b = OwnedIsolate::new();
    a.cx().call_out(|re| re.enter(|callback| std::mem::swap(callback, b.cx())));

    let ca = a.cx();
    std::mem::swap(ca, b.cx());
    drop(b);
    let _ = ca.nil();
}
