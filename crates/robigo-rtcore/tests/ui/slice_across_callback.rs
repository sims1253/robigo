// A slice of a vector does not survive a callback into the isolate.
use robigo_rtcore::OwnedIsolate;

fn main() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    let x = cx.alloc_copy(&[1.0, 2.0]).unwrap().finish();
    let xs: &[f64] = x.as_slice().unwrap();
    cx.call_out(|re| re.enter(|_cx| ()));
    let _ = xs[0];
}
