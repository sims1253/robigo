# Writes made by a callback while the code that called it is suspended
# (docs/g3.md §9.3, §15.3 E6): R code called back from package C
# (rt_callback) rebinds, removes, reclasses and rewrites what the
# suspended closure uses next. Each closure runs three times: its first
# call as GNU R's AST, the later ones past GNU R's JIT threshold as byte
# code; both resume with what GNU R resumes with.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/closure_mutation.so"))
cb <- function(f) .Call("rt_callback", f)
# a function the suspended code calls next, redefined
helper <- function() "old helper"
g1 <- function() {
  cb(function() helper <<- function() "new helper")
  r <- helper()
  helper <<- function() "old helper"
  r
}
print(g1()); print(g1()); print(g1())
# a local of the suspended frame, assigned and removed
g2 <- function() {
  v <- 1
  cb(function() v <<- 2)
  v
}
print(g2()); print(g2()); print(g2())
g3 <- function() {
  v <- 1
  e <- environment()
  cb(function() rm("v", envir = e))
  tryCatch(v, error = conditionMessage)
}
v <- "global v"
print(g3()); print(g3()); print(g3())
# the class of the object the suspended code dispatches on next
gen <- function(x) UseMethod("gen")
gen.a <- function(x) "method a"
gen.b <- function(x) "method b"
g4 <- function() {
  x <- structure(1, class = "a")
  first <- gen(x)
  cb(function() class(x) <<- "b")
  c(first, gen(x))
}
print(g4()); print(g4()); print(g4())
# the suspended closure's own default, replaced before its promise is
# forced: the promise made at the call keeps the old one, the next call
# promises the new
g5 <- function(x = "old default") {
  cb(function() .Call("rt_set_formal_default", g5, 0L, "new default"))
  x
}
print(g5()); print(g5()); print(g5())
# the default call written in place before its promise is forced: the
# promise evaluates the call it holds, so it sees the write
y <- 1
g6 <- function(x = y + 1) {
  cb(function() .Call("rt_set_default_elt", g6, 0L, 2L, 10 * (y + 1)))
  y <<- y + 1
  x
}
print(g6()); print(g6()); print(g6())
