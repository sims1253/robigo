//! `.Internal(inspect())` against GNU R 4.6.2's output
//! (`support/inspect.gnur.out`, recorded from `support/inspect.R` with
//! `R_DEFAULT_PACKAGES=NULL`, `LC_ALL=C.UTF-8`, `LANGUAGE=en`, `TZ=UTC`).
//!
//! The conformance suite compares a transcript byte for byte, and robigo-rt
//! cannot yet capture `inspect`'s output inside R to mask it there (no
//! `sink`, no output `textConnection`, no `tempdir`). So this test runs the
//! script and masks both transcripts the same way:
//! - an address (`@<hex>`, `<0x...>`), never compared (L1);
//! - `REF(n)`, which counts references from GNU R's own objects too (a
//!   symbol's thousands, R_NilValue's 65535): robigo's own.
//!
//! Everything else on each line is compared: the type, `g0c<class>`, every
//! flag, `gp=`, lengths and `tl=`, strings, names and elements, and the
//! tree's shape.

use std::path::Path;
use std::process::Command;

/// `line` with addresses and reference counts masked.
fn mask(line: &str) -> String {
    let b = line.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let hex_run = |from: usize| (from..b.len()).take_while(|&k| b[k].is_ascii_hexdigit()).count();
        if b[i] == b'@' {
            out.push('@');
            i += 1 + hex_run(i + 1);
        } else if line[i..].starts_with("<0x") {
            let n = hex_run(i + 3);
            out.push_str("<ptr>");
            i += 3 + n + 1;
        } else if line[i..].starts_with("REF(") {
            let digits = (i + 4..b.len()).take_while(|&k| b[k].is_ascii_digit()).count();
            i += 4 + digits + 1;
            if i < b.len() && b[i] == b',' {
                i += 1;
            }
        } else {
            let c = line[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out.replace(",]", "]")
}

#[test]
fn inspect_prints_gnu_rs_dump() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support");
    let out = Command::new(env!("CARGO_BIN_EXE_robigo-rt"))
        .args(["--vanilla", "--quiet", "--no-echo", "-f", "inspect.R"])
        .current_dir(&dir)
        .env("LANGUAGE", "en")
        .env("LC_ALL", "C.UTF-8")
        .env("TZ", "UTC")
        .env("R_DEFAULT_PACKAGES", "NULL")
        .env_remove("R_HOME")
        .output()
        .expect("run robigo-rt");
    let got = String::from_utf8_lossy(&out.stdout);
    let want = std::fs::read_to_string(dir.join("inspect.gnur.out")).unwrap();
    let got: Vec<String> = got.lines().map(mask).collect();
    let want: Vec<String> = want.lines().map(mask).collect();
    for (k, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(
            g,
            w,
            "line {}:\n robigo-rt: {g}\n GNU R:     {w}\n(stderr: {})",
            k + 1,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    assert_eq!(got.len(), want.len(), "line count (stderr: {})", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn the_mask_hides_addresses_and_counts_only() {
    assert_eq!(
        mask("@59784de09948 13 INTSXP g0c0 [REF(65535)]  1 : 6 (compact)"),
        "@ 13 INTSXP g0c0 []  1 : 6 (compact)"
    );
    assert_eq!(
        mask("  @1a 04 ENVSXP g0c0 [REF(2),GL,gp=0x8000] <0xdeadbeef>"),
        "  @ 04 ENVSXP g0c0 [GL,gp=0x8000] <ptr>"
    );
    assert_eq!(
        mask("@ff 13 INTSXP g0c1 [OBJ,REF(1),ATT] (len=2, tl=0) 1,2"),
        "@ 13 INTSXP g0c1 [OBJ,ATT] (len=2, tl=0) 1,2"
    );
}
