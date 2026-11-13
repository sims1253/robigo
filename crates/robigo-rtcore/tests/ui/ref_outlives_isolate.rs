// A `Ref` cannot outlive its isolate.
use robigo_rtcore::OwnedIsolate;

fn main() {
    let r = {
        let mut iso = OwnedIsolate::new();
        iso.cx().nil()
    };
    let _ = r.sexptype();
}
