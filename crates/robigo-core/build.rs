// The C shim: setjmp-based contexts and the register spill for the
// conservative stack scan (Rust cannot call setjmp).
fn main() {
    println!("cargo:rerun-if-changed=src/shim.c");
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
}
