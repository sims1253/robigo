// The C side of the contexts: RCNTXT's layout as robigo's Defn.h declares
// it (src/context.c), which the runtime checks its own declaration against.
fn main() {
    // Rerun when another checkout built last (.cargo/config.toml).
    println!("cargo:rerun-if-env-changed=ROBIGO_WORKTREE");
    println!("cargo:rerun-if-changed=src/context.c");
    println!("cargo:rerun-if-changed=include/Defn.h");
    println!("cargo:rerun-if-changed=src/test_routines.c");
    println!("cargo:rerun-if-changed=src/capi.c");
    println!("cargo:rerun-if-changed=include/robigo_capi.h");
    // robigo_layout.h, which robigo-heap's build writes (its `links`):
    // Defn.h reads it
    let layout = std::env::var("DEP_ROBIGO_HEAP_INCLUDE").unwrap_or_else(|e| {
        panic!("robigo-rtcore: DEP_ROBIGO_HEAP_INCLUDE: {e} (robigo-heap's `links` key and its `cargo:include` line set it)")
    });
    println!("cargo:rerun-if-changed={layout}/robigo_layout.h");
    cc::Build::new()
        .file("src/context.c")
        .file("src/capi.c")
        .include("include")
        .include(&layout)
        .flag_if_supported("-std=gnu11")
        .compile("robigo_rtcore_context");
    // C stand-ins for the tests (package routines, cends, cleanups), a
    // separate archive: unreferenced, it stays out of every binary but the
    // tests.
    cc::Build::new()
        .file("src/test_routines.c")
        .include("include")
        .include(&layout)
        .flag_if_supported("-std=gnu11")
        .compile("robigo_rtcore_test_routines");
}
