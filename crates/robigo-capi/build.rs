// libR.so carries the soname package objects ask for (DT_NEEDED libR.so).
// The bridge also embeds a soname-only stub libR.so: objects built with
// GNU R's Makeconf (-lR) get it satisfied, and their API symbols bind to
// the robigo executable's exports (docs/core.md §6).
use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libR.so");
    println!("cargo:rerun-if-changed=build.rs");
    // GNU R's src/appl C code, built into libR as GNU R does (appl/README.md)
    println!("cargo:rerun-if-changed=appl");
    cc::Build::new()
        .files(["appl/optim.c", "appl/uncmin.c", "appl/integrate.c"])
        .include("appl")
        .include("include")
        .define("HAVE_CONFIG_H", None)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-sign-compare")
        .flag_if_supported("-Wno-maybe-uninitialized")
        .flag_if_supported("-Wno-unused-but-set-variable")
        .compile("robigo_appl");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let src = out.join("libR-stub.c");
    std::fs::write(&src, "/* robigo's stub libR.so: a soname, no code */\nint robigo_libR_stub;\n").unwrap();
    let so = out.join("libR-stub.so");
    let compiler = cc::Build::new().get_compiler();
    let st = compiler
        .to_command()
        .args(["-shared", "-fPIC", "-Wl,-soname,libR.so", "-o"])
        .arg(&so)
        .arg(&src)
        .status()
        .expect("running the C compiler for the libR.so stub");
    assert!(st.success(), "building the libR.so stub failed");
}
