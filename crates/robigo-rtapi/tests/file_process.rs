//! File connections across the process, against GNU R 4.6.2 recorded in
//! the oracle image with the environment the conformance suites use; a
//! conformance case cannot set these up, so they are here.
//!
//! - A write after a failed flush: under a file size limit (`ulimit -f 8`,
//!   8192 bytes, with SIGXFSZ ignored, so a write past it fails with
//!   EFBIG), an append connection fills the file, a third string's flush
//!   fails (the buffer's bytes are dropped, writeLines raises), another
//!   connection empties the file, and the next write goes to the new end.
//!   Recorded with `scripts/oracle-run.sh 4.6.2 bash -c 'trap "" XFSZ;
//!   ulimit -f 8; Rscript --vanilla -' < script`.
//! - Exit: a connection left open has its buffer written when the process
//!   exits (glibc's `_IO_cleanup`), at the script's end, at `q()` and after
//!   an error. GNU R 4.6.2 leaves "hello\n" in the file in each.

use std::process::Command;

const LIMIT_SCRIPT: &str = r#"f <- tempfile()
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
    std::fs::write(&script, LIMIT_SCRIPT).unwrap();
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

#[test]
fn a_connection_left_open_is_written_at_exit() {
    let dir = std::env::temp_dir().join(format!("robigo-file-exit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // (how the script ends, its status)
    for (end, tail, status) in [("end", "", 0), ("quit", "q(status = 3)", 3), ("error", "stop(\"boom\")", 1)] {
        let out = dir.join(format!("out-{end}"));
        let script = dir.join(format!("{end}.R"));
        let src = format!(
            "con <- file(\"{}\", \"w\")\nwriteLines(\"hello\", con)\ncat(\"written\\n\")\n{tail}\n",
            out.display()
        );
        std::fs::write(&script, src).unwrap();
        let r = Command::new(env!("CARGO_BIN_EXE_robigo-rt"))
            .arg(&script)
            .env("LANGUAGE", "en")
            .env("LC_ALL", "C.UTF-8")
            .env("TZ", "UTC")
            .env("R_DEFAULT_PACKAGES", "NULL")
            .env_remove("R_HOME")
            .output()
            .expect("run robigo-rt");
        assert_eq!(String::from_utf8_lossy(&r.stdout), "written\n", "{end}");
        assert_eq!(r.status.code(), Some(status), "{end}: {}", String::from_utf8_lossy(&r.stderr));
        assert_eq!(std::fs::read(&out).unwrap(), b"hello\n", "{end}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
