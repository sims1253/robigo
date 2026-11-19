// An environment writer stays on its isolate's thread: `EnvWrites::assume`'s
// contract counts on the writes happening there, so `EnvWriter` is neither
// `Sync` (a scoped thread cannot borrow it) nor `Send`.
fn need_sync<T: Sync>(_: &T) {}

fn main() {
    let mut iso = robigo_rtcore::OwnedIsolate::new();
    if let Ok(w) = robigo_rtcore::clib::env_writer(iso.cx()) {
        need_sync(&w);
    }
}
