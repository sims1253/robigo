// The C shim: setjmp-based contexts and the register spill for the
// conservative stack scan (Rust cannot call setjmp).
fn main() {
    println!("cargo:rerun-if-changed=src/shim.c");
    cc::Build::new().file("src/shim.c").flag_if_supported("-fno-omit-frame-pointer").compile("robigo_core_shim");
}
