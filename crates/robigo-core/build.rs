// The C shim: setjmp-based contexts and the register spill for the
// conservative stack scan (Rust cannot call setjmp).
fn main() {
    // Rerun when another checkout built last (.cargo/config.toml).
    println!("cargo:rerun-if-env-changed=ROBIGO_WORKTREE");
    println!("cargo:rerun-if-changed=src/shim.c");
    println!("cargo:rerun-if-changed=src/test_routines.c");
    // No unwind tables, deliberately: Rust imports these functions as
    // `extern "C"` (no unwinding), so an exception must not get past them.
    // The unwinder's search phase stops at a frame without tables, so a C++
    // `throw` no `catch` below the shim handles ends in `std::terminate`
    // instead of finding a `catch` above Rust frames (docs/core.md §5).
    cc::Build::new()
        .file("src/shim.c")
        .flag_if_supported("-fno-omit-frame-pointer")
        .flag_if_supported("-fno-unwind-tables")
        .flag_if_supported("-fno-asynchronous-unwind-tables")
        .compile("robigo_core_shim");
    // Package-routine stand-ins for tests/core.rs, a separate archive
    // member: unreferenced, it stays out of every binary but the tests.
    cc::Build::new().file("src/test_routines.c").compile("robigo_core_test_routines");
}
