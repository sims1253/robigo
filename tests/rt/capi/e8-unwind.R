# E8 across a C frame (docs/g3.md §15.3), recorded on GNU R 4.6.2: on.exit
# code that catches its own error, an R_UnwindProtect cleanup that catches,
# and a C context's cend before the jump. Its C is support/e8_unwind.c,
# built by scripts/rt-capi-build.sh into RT_CAPI_LIBS. The Rust tests of
# crates/robigo-rtapi/tests/capi.rs (E8) assert the same.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/e8_unwind.so"))
msg <- function(expr) tryCatch({ force(expr); "no error" }, error = function(e) conditionMessage(e))

# The three cases, run by a closure that GNU R's JIT runs as the AST at
# its first call and as byte code from its second (R_CheckJIT): GNU R's
# own messages differ between the two (getCurrentCall reads the byte-code
# interpreter's expression), so both are recorded.
f <- function() {
  on.exit(.Call("e8_catch_inner"))
  .Call("e8_error", "outer")
}
cases <- function() {
  # 1: f's on.exit code catches an error of its own during the unwind of
  # f's error, with a C routine's frame between the error and the
  # catcher: the condition that leaves is the outer one
  print(msg(.Call("e8_eval_call", f)))
  # 2: an R_UnwindProtect cleanup that catches an error of its own; the
  # original exit resumes and reaches the catcher
  print(msg(.Call("e8_unwind_catching_cleanup")))
  # 3: R_ExecWithCleanup's cleanup runs before the jump, while the
  # routine's frame is still there
  print(msg(.Call("e8_exec_with_cleanup_fails")))
}
cases()
cases()
cases()
