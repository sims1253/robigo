// robigo-rtapi's C half (src/api.c: accessors, variadic functions,
// R_UnwindProtect), and the link flag that exports the API from the
// binaries: packages loaded with dlopen resolve GNU R's symbols against the
// executable (docs/g3.md §14.1).
fn main() {
    // Rerun when another checkout built last (.cargo/config.toml).
    println!("cargo:rerun-if-env-changed=ROBIGO_WORKTREE");
    println!("cargo:rerun-if-changed=src/api.c");
    println!("cargo:rerun-if-changed=../robigo-rtcore/include/robigo_capi.h");
    // No unwind tables, as the heap's shim: nothing may unwind through C
    // frames of the API.
    cc::Build::new()
        .file("src/api.c")
        .include("../robigo-rtcore/include")
        .flag_if_supported("-std=gnu11")
        .flag_if_supported("-fno-unwind-tables")
        .flag_if_supported("-fno-asynchronous-unwind-tables")
        .compile("robigo_rtapi_c");
    // The API's symbols in the executables' dynamic symbol table.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-rdynamic");
        println!("cargo:rustc-link-arg-tests=-rdynamic");
    }
}
