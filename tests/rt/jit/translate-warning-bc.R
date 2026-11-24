# A name given as a string that does not translate to the native encoding
# (LC_ALL=C, the .locale sidecar), in closures the JIT compiles. cmp.R
# installs the name of `"x" <- v` itself: its warnings come at the compile,
# from cmp.R's own calls (findLocals' addVar, `assign(v, 1, envir = vars)`,
# once per pass; cmpAssign's `as.name(getAssignedVar(e, cntxt))`), and the
# compiled code gives none. `missing("x")` is left to the special, which
# warns at each evaluation, compiled or not.
options(warn = 1)
f <- function() { "\u4e2d" <- 1; ls() }
for (i in 1:3) { message("f ", i); print(f()) }
g <- function() missing("\u4e2d")
for (i in 1:3) { message("g ", i); print(tryCatch(g(), error = conditionMessage)) }
# a loop: compiled at its first call
h <- function(x) { for (i in 1:2) "\u4e2d" <- x; get("<U+4E2D>") }
message("h"); h(1); h(2)
# `=`, `<<-` (not a local: no addVar), and assign() of a string name, whose
# own installTrChar warns when it runs
k <- function() { "\u4e2d" = 1; "\u4e2d" <<- 2; assign("\u4e2d", 3); for (i in 1) NULL; ls() }
message("k"); k(); k()
# a nested function, compiled with the closure that makes it
m <- function() { n <- function() { "\u4e2d" <- 1; ls() }; for (i in 1) NULL; n() }
message("m"); m(); m()
# a handler of findLocals' warnings runs before the body is compiled: its
# `+` is the one the compile sees
f <- function(x) { "\u4e2d" <- 1; for (i in 1) NULL; x + 1 }
withCallingHandlers(print(f(1)), warning = function(w) {
  if (!exists("+", envir = globalenv(), inherits = FALSE))
    assign("+", function(e1, e2) 99, envir = globalenv())
  invokeRestart("muffleWarning")
})
print(f(1))
rm("+", envir = globalenv())
# ... and before the compile forces anything: the delayed binding the
# compile would read is replaced by the handler, never forced
delayedAssign("nchar", { message("forced"); base::nchar })
fd <- function(x) { "\u4e2d" <- 1; for (i in 1) NULL; nchar(x) }
withCallingHandlers(print(fd("abc")), warning = function(w) {
  if (exists("nchar", envir = globalenv(), inherits = FALSE)) rm("nchar", envir = globalenv())
  assign("nchar", function(x) 42, envir = globalenv())
  invokeRestart("muffleWarning")
})
print(fd("abc"))
rm("nchar", envir = globalenv())
# ... and findLocals' decisions are those made before the handler ran:
# `local` was base's, so the `if` it assigns is no local
fs <- function() {
  "\u4e2d" <- 1
  local(`if` <- function(a, b, c) c)
  for (i in 1) NULL
  if (TRUE) 1 else 2
}
withCallingHandlers(print(fs()), warning = function(w) {
  assign("local", function(x) x, envir = globalenv())
  invokeRestart("muffleWarning")
})
print(fs())
rm("local", envir = globalenv())
# a classed string target: no dispatch, the first string with the warning
as.vector.foo <- function(x, mode) stop("unexpected")
g <- eval(substitute(function() { S <- 1; ls() }, list(S = structure("\u4e2d", class = "foo"))))
print(g())
gm <- eval(substitute(function() missing(S), list(S = structure(c("\u4e2d", "b"), class = "foo"))))
print(tryCatch(gm(), error = conditionMessage))
# the JIT is off while a compile's warnings run
h <- function() { "\u4e2d" <- 1; for (i in 1) NULL; 0 }
withCallingHandlers(h(), warning = function(w) {
  message("jit ", .Internal(enableJIT(-1L)))
  invokeRestart("muffleWarning")
})
message("jit ", .Internal(enableJIT(-1L)))
# a top-level loop (compiled as it starts)
message("loop")
for (i in 1:2) "\u4e2d" <- i
get("<U+4E2D>")
