// Resetting a root needs `&mut Cx`, which ends the `Ref` read from it.
use robigo_rtcore::OwnedIsolate;

fn main() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let root = s.root(cx, cx.alloc_copy(&[1.0]).unwrap().finish());
        let x = root.bind(cx);
        let _out = root.reset(cx);
        let _ = x.sexptype();
    });
}
