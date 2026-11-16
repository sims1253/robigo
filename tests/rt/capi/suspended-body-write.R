# A callback writing what the closure that called it runs next, while that
# closure is suspended in the callback (docs/g3.md §9.3: "a suspended
# compiled activation whose assumption broke deopts at its next
# uncommitted operation"). GNU R's AST reads the next statement of a `{`
# from the live list after each statement (do_begin), a loop's body (read
# when the loop started) at each iteration, and a `while` condition at
# each iteration: the write takes effect at the next such boundary. Byte
# code is an object of its own: it keeps running what it was compiled
# from. robigo-rt's `Eval` code checks at those boundaries (Op::Resume,
# Op::LoopResume) and, when the code is stale, runs the rest as the AST
# does; `Bc` code has no checks.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/closure_mutation.so"))
cb <- function(f) .Call("rt_callback", f)
# a body GNU R never compiles (it may call browser()): always the AST
nb <- function() {
  if (FALSE) browser()
  cb(function() .Call("rt_set_body_elt", nb, 3L, "new statement"))
  "old statement"
}
print(nb())
print(nb())
# a small closure: its first call is the AST, the second compiles the body
# the first call's callback wrote
s <- function() {
  cb(function() .Call("rt_set_body_elt", s, 2L, "new statement"))
  "old statement"
}
print(s())
invisible(.Call("rt_set_body_elt", s, 2L, "old statement"))
print(s())
print(s())
# a closure compiled at its first call (its loop scores high): its byte
# code keeps running after the callback's write, at every call
bl <- function() {
  for (i in 1) NULL
  cb(function() .Call("rt_set_body_elt", bl, 3L, "new statement"))
  "old statement"
}
print(bl())
print(bl())
step <- function(i, f) {
  if (i == 1L) cb(f)
  i
}
# a for loop's body (a call) written in its first iteration: each later
# iteration evaluates the written body
lf <- function() {
  if (FALSE) browser()
  acc <- 0
  for (i in 1:3) acc <- acc + step(i, function() .Call("rt_set_path", lf, c(3L, 3L, 2L, 0L), quote(`-`)))
  acc
}
print(lf())
# a while loop's condition written in an iteration: the next test reads it
wf <- function() {
  if (FALSE) browser()
  k <- 0
  while (k < 10) k <- k + step(1L, function() .Call("rt_set_path", wf, c(3L, 1L, 2L), 3))
  k
}
print(wf())
# a statement of a loop's `{` written by the statement before it: the same
# iteration runs the written one
bf <- function() {
  if (FALSE) browser()
  out <- 0
  for (i in 1:3) {
    out <- out + step(i, function() .Call("rt_set_path", bf, c(3L, 3L, 2L), quote(out <- out * 10)))
    out <- out + 0
  }
  out
}
print(bf())
