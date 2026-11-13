// A `Ref` does not survive `&mut Cx`: foreign code may collect.
use robigo_rtcore::OwnedIsolate;

fn main() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    let x = cx.alloc_copy(&[1.0]).unwrap().finish();
    cx.call_out(|_| ());
    let _ = x.len();
}
