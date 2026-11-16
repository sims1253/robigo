# Which of NA and NaN an arithmetic operation keeps when both meet (on
# x86-64 the destination operand's), on each of GNU R's paths. The AST's
# do_arith scalar block makes y the destination of `+` and `*` for a
# scalar double against a scalar integer; byte code makes it x (an NA
# integer leaves Arith2's fast path for cmp_arith2, which calls R_binary's
# mixed-type loop). Seen at top level (do.call: the AST), in closures GNU R
# never compiles, in compiled closures, in callees of a compiled caller
# (which robigo-rt may run inline), in folded constants, and in a top-level
# loop, which GNU R's JIT compiles.
show <- function(v) paste(v, collapse = " ")
pairs <- list(list(NaN, NA_integer_), list(NA_integer_, NaN), list(NaN, NA), list(NA, NaN),
              list(NA_real_, NaN), list(NaN, NA_real_), list(c(NaN, 1), NA_integer_), list(NA_integer_, c(NaN, 1)),
              list(NaN, c(NA_integer_, 1L)), list(c(NA_integer_, 1L), NaN), list(NA_integer_, NA_integer_))
ops <- c("+", "-", "*", "/", "^", "%%", "%/%")
run <- function(f) {
  out <- character()
  for (p in pairs) out <- c(out, show(f(p[[1L]], p[[2L]])))
  paste(out, collapse = " | ")
}
# the AST: do.call evaluates the call as eval does
for (op in ops) cat("ast     ", op, run(function(a, b) do.call(op, list(a, b))), "\n")
# closures made by a function called once: GNU R never compiles them
mkall <- function() list(`+` = function(a, b) a + b, `-` = function(a, b) a - b, `*` = function(a, b) a * b,
                         `/` = function(a, b) a / b, `^` = function(a, b) a ^ b, `%%` = function(a, b) a %% b,
                         `%/%` = function(a, b) a %/% b)
nojit <- mkall()
for (op in ops) cat("nojit   ", op, run(nojit[[op]]), "\n")
# top-level closures: compiled at their second call
cf <- list(`+` = function(a, b) a + b, `-` = function(a, b) a - b, `*` = function(a, b) a * b,
           `/` = function(a, b) a / b, `^` = function(a, b) a ^ b, `%%` = function(a, b) a %% b,
           `%/%` = function(a, b) a %/% b)
for (op in ops) {
  f <- cf[[op]]
  for (k in 1:3) r <- run(f)
  cat("compiled", op, r, "\n")
}
# a compiled caller (its loop compiles it at its first call) and small
# callees: compiled ones (from their second call) and ones GNU R never
# compiles
add <- function(a, b) a + b
sub <- function(a, b) a - b
mul <- function(a, b) a * b
dvd <- function(a, b) a / b
pw <- function(a, b) a ^ b
nadd <- nojit[["+"]]
nmul <- nojit[["*"]]
calls <- function(x, y) {
  r <- NULL
  for (i in 1:3) r <- c(add(x, y), add(y, x), sub(x, y), mul(x, y), mul(y, x), dvd(x, y), pw(x, y),
                        nadd(x, y), nadd(y, x), nmul(x, y), nmul(y, x))
  r
}
for (k in 1:2) cat("callees ", show(calls(NaN, NA_integer_)), "|", show(calls(NA_real_, NaN)), "\n")
# constants in compiled code, which the compiler folds as eval computes them
cst <- function() c(NaN + NA_integer_, NA_integer_ + NaN, NaN * NA_integer_, NA_integer_ * NaN, NaN - NA_integer_,
                    NA_integer_ - NaN, NaN / NA_integer_, NA_integer_ ^ NaN)
for (k in 1:3) r <- cst()
cat("folded  ", show(r), "\n")
# a top-level loop
a <- NaN
b <- NA_integer_
for (i in 1:2) r <- c(a + b, b + a, a * b, b * a, a - b, b - a, a / b, b / a, a ^ b, b ^ a)
cat("toploop ", show(r), "\n")
