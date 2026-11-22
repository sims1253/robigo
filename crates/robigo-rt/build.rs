// libdeflate's zlib decompressor (vendor/libdeflate, MIT): what GNU R's
// R_decompress1 calls for the lazy-load databases' records
// (serialize::inflate). Built with upstream's release flags (CMakeLists.txt:
// `-O2 -DNDEBUG`), fixed here rather than taken from cargo's profile, so the
// decoder's instructions are the same in every profile the perf rows build.
fn main() {
    // Rerun when another checkout built last (.cargo/config.toml).
    println!("cargo:rerun-if-env-changed=ROBIGO_WORKTREE");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/libdeflate");
    println!("cargo:rerun-if-changed={}", dir.display());
    let lib = dir.join("lib");
    cc::Build::new()
        .file(lib.join("deflate_decompress.c"))
        .file(lib.join("zlib_decompress.c"))
        .file(lib.join("adler32.c"))
        .file(lib.join("utils.c"))
        // (each compiles to nothing on the other architectures)
        .file(lib.join("x86/cpu_features.c"))
        .file(lib.join("arm/cpu_features.c"))
        .include(&dir)
        .opt_level(2)
        .define("NDEBUG", None)
        .flag_if_supported("-std=gnu99")
        // (upstream's sources, unmodified: their warnings are upstream's)
        .warnings(false)
        .compile("robigo_libdeflate");
}
