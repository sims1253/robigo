// An isolate's capability and handles stay on its thread.
use robigo_rtcore::{Cx, Global, OwnedIsolate, Ref, Reentry, Root};

fn need_send<T: ?Sized + Send>() {}

fn main() {
    need_send::<Cx>();
    need_send::<OwnedIsolate>();
    need_send::<Ref<'static>>();
    need_send::<Root<'static>>();
    need_send::<Global>();
    need_send::<Reentry<'static>>();
}
