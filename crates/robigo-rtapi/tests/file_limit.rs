//! A file connection whose flush fails and whose next write succeeds:
//! under a file size limit (`ulimit -f 8`, 8192 bytes, with SIGXFSZ
//! ignored, so a write past it fails with EFBIG), an append connection
//! fills the file, a third string's flush fails (the buffer's bytes are
//! dropped, writeLines raises), another connection empties the file, and
//! the next write goes to the new end. The expected output is GNU R 4.6.2's,
//! recorded in the oracle image (`scripts/oracle-run.sh 4.6.2 bash -c 'trap
//! "" XFSZ; ulimit -f 8; Rscript --vanilla -' < script`) with the
//! environment the conformance suites use. A conformance case cannot set
//! the limit, so it is here.

use std::process::Command;

const SCRIPT: &str = r#"f <- tempfile()
con <- file(f, "a")
writeLines(strrep("a", 5000), con)
print(file.size(f))
writeLines(strrep("b", 5000), con)
print(file.size(f))
print(tryCatch(writeLines(strrep("c", 5000), con), error = conditionMessage))
print(file.size(f))
close(file(f, "w"))
print(file.size(f))
writeLines("after", con)
print(file.size(f))
print(close(con))
print(readLines(f))
invisible(file.remove(f))
"#;

const GNU_R: &str = r#"[1] 4096
[1] 8192
[1] "Error writing to connection:  File too large"
[1] 8192
[1] 0
[1] 0
[1] 0
[1] "after"
"#;

#[test]
fn a_write_after_a_failed_flush_succeeds_and_the_dropped_bytes_are_gone() {
    let dir = std::env::temp_dir().join(format!("robigo-file-limit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("fsize.R");
    std::fs::write(&script, SCRIPT).unwrap();
    // the limit is the shell's, inherited by robigo-rt; robigo-rt's own
    // writes stay below it (the script's tempfile is the one that grows)
    let out = Command::new("bash")
        .args(["-c", "trap '' XFSZ; ulimit -f 8; exec \"$0\" \"$1\""])
        .arg(env!("CARGO_BIN_EXE_robigo-rt"))
        .arg(&script)
        .env("LANGUAGE", "en")
        .env("LC_ALL", "C.UTF-8")
        .env("TZ", "UTC")
        .env("R_DEFAULT_PACKAGES", "NULL")
        .env_remove("R_HOME")
        .output()
        .expect("run robigo-rt under bash");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(String::from_utf8_lossy(&out.stdout), GNU_R, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(0));
}
