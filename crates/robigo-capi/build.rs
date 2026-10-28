// libR.so must carry the soname package objects ask for (DT_NEEDED
// libR.so), so that glibc binds them to the copy robigo has loaded.
fn main() {
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libR.so");
}
