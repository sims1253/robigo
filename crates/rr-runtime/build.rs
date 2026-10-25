//! Embed the vendored base R sources, in the order GNU R's build uses
//! (`LC_COLLATE=C ls R/*.R R/unix/*.R`).

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../rlib");
    let mut files: Vec<PathBuf> = Vec::new();
    for dir in ["base/R", "base/R/unix"] {
        let d = root.join(dir);
        println!("cargo:rerun-if-changed={}", d.display());
        let mut v: Vec<PathBuf> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "R"))
            .collect();
        v.sort_by(|a, b| a.file_name().unwrap().as_encoded_bytes().cmp(b.file_name().unwrap().as_encoded_bytes()));
        files.extend(v);
    }
    let mut out = String::from("&[\n");
    for f in &files {
        let canon = f.canonicalize().unwrap();
        println!("cargo:rerun-if-changed={}", canon.display());
        let name = canon.strip_prefix(root.canonicalize().unwrap()).unwrap();
        out.push_str(&format!("    ({:?}, include_str!({:?})),\n", name.display().to_string(), canon.display().to_string()));
    }
    out.push_str("]\n");
    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("base_sources.rs");
    std::fs::write(dest, out).unwrap();
}
