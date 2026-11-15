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
    // The API's symbols in the executables' dynamic symbol table, which a
    // package dlopen'ed later resolves them against. Without it every
    // package fails to load with undefined symbols, so a target this does
    // not know says so at build time.
    let export = match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("linux") => Some("-rdynamic"),
        Ok("macos") => Some("-Wl,-export_dynamic"),
        _ => None,
    };
    match export {
        Some(flag) => {
            println!("cargo:rustc-link-arg-bins={flag}");
            println!("cargo:rustc-link-arg-tests={flag}");
        }
        None => println!(
            "cargo:warning=robigo-rtapi: no flag to export the C API from executables on this target; packages will not load"
        ),
    }
}
