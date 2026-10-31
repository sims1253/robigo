//! Interactive shutdown paths that the non-interactive conformance runner
//! cannot reach: `q()` and EOF run `.Last`, and a `.Last` error returns to
//! the REPL instead of printing "Execution halted" (R: main.c:R_dot_Last).
//!
//! `--interactive` with piped stdin is the interactive REPL; it avoids a
//! PTY while still exercising the same `repl()` path. The EOF-error case
//! (which returns to the prompt and waits for more input) needs a terminal
//! and is checked with a controlled PTY in the PR.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(input: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_robigo"))
        .args(["--vanilla", "--quiet", "--interactive"])
        .env("R_DEFAULT_PACKAGES", "NULL")
        .env("LANGUAGE", "en")
        .env("LC_ALL", "C.UTF-8")
        .env("TZ", "UTC")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn robigo");
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn interactive_q_last_error_returns_to_repl() {
    let out = run(".Last <- function() stop(\"last failed\")\n\
         reg.finalizer(new.env(), function(e) cat(\"fin\\n\"), onexit = TRUE)\n\
         q(); cat(\"after\\n\")\n\
         q(\"no\", 7, FALSE)\n");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(7), "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stderr.contains("Error in .Last() : last failed"), "stderr:\n{stderr}");
    assert!(!stderr.contains("Execution halted"), "stderr:\n{stderr}");
    // The rest of the q() input line is dropped, like R's top-level error.
    assert!(!stdout.contains("after"), "stdout:\n{stdout}");
    // The aborted quit ran no finalizer; the explicit q(runLast = FALSE) did.
    assert_eq!(stdout.matches("fin\n").count(), 1, "stdout:\n{stdout}");
}

#[test]
fn interactive_eof_runs_last_then_finalizers() {
    let out = run(".Last <- function() cat(\"last\\n\")\n\
         reg.finalizer(new.env(), function(e) cat(\"fin\\n\"), onexit = TRUE)\n");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("last\n"), "stdout:\n{stdout}");
    assert!(stdout.contains("fin\n"), "stdout:\n{stdout}");
    assert!(!stderr.contains("Execution halted"), "stderr:\n{stderr}");
}

#[test]
fn interactive_reports_true() {
    let out = run("cat(interactive(), \"\\n\")\nq(\"no\", 0, FALSE)\n");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stdout:\n{stdout}");
    assert!(stdout.contains("TRUE"), "stdout:\n{stdout}");
}

#[test]
fn interactive_q_last_error_skips_caller_on_exit() {
    let out = run(".Last <- function() stop(\"last failed\")\n\
         f <- function() { on.exit(return(invisible(NULL))); q(\"no\") }\n\
         f(); cat(\"AFTER\\n\")\n\
         q(\"no\", 7, FALSE)\n");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(7), "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stderr.contains("Error in .Last() : last failed"), "stderr:\n{stderr}");
    // R's top-level context skips f's on.exit and drops the rest of the line.
    assert!(!stdout.contains("AFTER"), "stdout:\n{stdout}");
}

#[test]
fn interactive_is_set_before_first() {
    let profile = std::env::temp_dir().join(format!("robigo-first-{}.Rprofile", std::process::id()));
    std::fs::write(&profile, ".First <- function() cat(\"first\", interactive(), \"\\n\")\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_robigo"))
        .args(["--quiet", "--interactive"])
        .env("R_DEFAULT_PACKAGES", "NULL")
        .env("R_PROFILE_USER", &profile)
        .env("LANGUAGE", "en")
        .env("LC_ALL", "C.UTF-8")
        .env("TZ", "UTC")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn robigo");
    child.stdin.take().unwrap().write_all(b"q(\"no\", 0, FALSE)\n").unwrap();
    let out = child.wait_with_output().unwrap();
    let _ = std::fs::remove_file(&profile);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stdout:\n{stdout}");
    assert!(stdout.contains("first TRUE"), "stdout:\n{stdout}");
}
