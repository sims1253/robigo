// A `Ref` bound inside a scope cannot leave it.
use robigo_rtcore::OwnedIsolate;

fn main() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    let r = cx.scope(|cx, s| {
        let root = s.root(cx, cx.nil());
        root.bind(cx)
    });
    let _ = r.sexptype();
}
