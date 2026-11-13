// A `Root` cannot leave its scope, whose end frees its slot.
use robigo_rtcore::OwnedIsolate;

fn main() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    let root = cx.scope(|cx, s| s.root(cx, cx.nil()));
    let _ = root.bind(cx);
}
