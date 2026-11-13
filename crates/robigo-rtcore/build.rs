// The C side of the contexts: RCNTXT's layout as robigo's Defn.h declares
// it (src/context.c), which the runtime checks its own declaration against.
fn main() {
    // Rerun when another checkout built last (.cargo/config.toml).
    println!("cargo:rerun-if-env-changed=ROBIGO_WORKTREE");
    println!("cargo:rerun-if-changed=src/context.c");
    println!("cargo:rerun-if-changed=include/Defn.h");
    println!("cargo:rerun-if-changed=src/test_routines.c");
    cc::Build::new()
        .file("src/context.c")
        .include("include")
        .flag_if_supported("-std=gnu11")
        .compile("robigo_rtcore_context");
    // C stand-ins for the tests (package routines, cends, cleanups), a
    // separate archive: unreferenced, it stays out of every binary but the
    // tests.
    cc::Build::new()
        .file("src/test_routines.c")
        .include("include")
        .flag_if_supported("-std=gnu11")
        .compile("robigo_rtcore_test_routines");
}
