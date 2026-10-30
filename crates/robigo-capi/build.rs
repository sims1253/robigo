// libR.so carries the soname package objects ask for (DT_NEEDED libR.so).
// The bridge also embeds a soname-only stub libR.so: objects built with
// GNU R's Makeconf (-lR) get it satisfied, and their API symbols bind to
// the robigo executable's exports (docs/core.md §6).
use std::path::PathBuf;

fn main() {
    // The target platform: Linux keeps GNU R's exact link shape (ELF
    // soname, --whole-archive, libz/libbz2 like GNU R's libR). macOS
    // uses its own install-name/dynamiclib shape; Windows is not
    // supported yet (the runtime uses Unix process and fifo APIs).
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let is_linux = target_os == "linux";
    let is_macos = target_os == "macos";
    if is_linux {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libR.so");
    } else if is_macos {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-install_name,@rpath/libR.dylib");
    }
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
    // GNU R's graphics engine (main/README.md): its own sources and
    // internal headers, linked whole so every engine function is in the
    // executable for the graphics packages' shared objects
    println!("cargo:rerun-if-changed=main");
    let mut g = cc::Build::new();
    for f in [
        "engine",
        "devices",
        "plotmath",
        "patterns",
        "clippath",
        "mask",
        "colors",
        "gevents",
        "plot3d",
        "graphics",
        "format",
        "rlocale",
        "robigo_print",
        "plot",
        "g_alab_her",
        "g_cntrlify",
        "g_fontdb",
        "g_her_glyph",
        "localecharset",
        "robigo_glue",
        "robigo_sys",
        "robigo_hashtab",
        "robigo_util",
        "robigo_internet",
        "Rsock",
        "sock",
        "dounzip",
    ] {
        g.file(format!("main/{f}.c"));
    }
    g.file("appl/pretty.c")
        .include("main/include")
        .include("main")
        .include("include")
        .define("HAVE_CONFIG_H", None)
        .define("TESTING_WRITE_BARRIER", None)
        .flag_if_supported("-std=gnu2x")
        .flag_if_supported("-w");
    // Whole-archive keeps every engine function in the binary for the
    // graphics packages' shared objects. ELF-only for now; macOS links
    // the archive normally (native graphics-package loading there is a
    // later track, like the executable's export list).
    if is_linux {
        g.link_lib_modifier("+whole-archive");
    }
    g.compile("robigo_graphics");
    // zlib, for gzio.h (and the graphics packages' objects, as GNU R's
    // libR links it)
    if !target_os.is_empty() && target_os != "windows" {
        println!("cargo:rustc-link-lib=z");
    }
    // libbz2, for dounzip.c (bzip2 zip entries), as GNU R's libR links it
    if is_linux || is_macos {
        println!("cargo:rustc-link-lib=bz2");
    }
    // The soname-only stub libR.so that package objects link against
    // (-lR): ELF on Linux, a dynamiclib stub on macOS, skipped where the
    // runtime is not supported yet.
    if !(is_linux || is_macos) {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let src = out.join("libR-stub.c");
    std::fs::write(&src, "/* robigo's stub libR.so: a soname, no code */\nint robigo_libR_stub;\n").unwrap();
    let compiler = cc::Build::new().get_compiler();
    let st = if is_linux {
        let so = out.join("libR-stub.so");
        compiler
            .to_command()
            .args(["-shared", "-fPIC", "-Wl,-soname,libR.so", "-o"])
            .arg(&so)
            .arg(&src)
            .status()
            .expect("running the C compiler for the libR.so stub")
    } else {
        let so = out.join("libR-stub.dylib");
        compiler
            .to_command()
            .args(["-dynamiclib", "-Wl,-install_name,@rpath/libR.dylib", "-o"])
            .arg(&so)
            .arg(&src)
            .status()
            .expect("running the C compiler for the libR.dylib stub")
    };
    assert!(st.success(), "building the libR stub failed");
}
