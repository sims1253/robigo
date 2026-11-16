# check_stack_balance on a loop the AST runs from the live code: a callback
# rewrites what a suspended loop evaluates next (support/ppleak.c's
# pp_set_path), a `while` condition do_while reads at each iteration
# (CAR(args)) and a statement of the body a loop evaluates at each
# iteration (the object it read when it started). robigo-rt's Op::Resume
# and Op::LoopResume run those as the AST does, with do_for's, do_while's
# and do_begin's depths.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/ppleak.so"))
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/closure_mutation.so"))
mark <- function(s) cat("##", s, "\n", file = stderr())
cb <- function(f) .Call("rt_callback", f)
# bodies GNU R never compiles (they may call browser()): always the AST
wl <- function() {
  if (FALSE) browser()
  n <- 0
  while (n < 1) {
    n <- n + 1
    cb(function() .Call("pp_set_path", wl, c(4L, 2L), quote(is.null(.Call("leak")) && n < 3)))
  }
  n
}
mark("while-condition"); wl()
fl <- function() {
  if (FALSE) browser()
  for (i in 1:2) {
    cb(function() .Call("pp_set_path", fl, c(3L, 4L, 3L), quote(invisible(.Call("leak")))))
    "old statement"
  }
}
mark("for-body"); fl()
rl <- function() {
  if (FALSE) browser()
  k <- 0
  repeat {
    k <- k + 1
    if (k > 2) break
    cb(function() .Call("pp_set_path", rl, c(4L, 2L, 4L), quote(if (k > 2) break else invisible(.Call("leak")))))
  }
  k
}
mark("repeat-body"); rl()
mark("end")
