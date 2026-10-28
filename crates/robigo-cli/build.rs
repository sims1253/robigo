// The robigo executable exports GNU R's C API (robigo-capi) to the shared
// objects it loads, and nothing else (docs/core.md §6).
fn main() {
    let list = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../robigo-capi/exports.list");
    println!("cargo:rerun-if-changed={}", list.display());
    println!("cargo:rustc-link-arg-bins=-Wl,--dynamic-list={}", list.display());
}
