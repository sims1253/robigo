# getBindingType of a closure's arguments before and after their first
# read (docs/g3.md §15.3 E5): a constant, a symbol, a call and a defaulted
# argument, with the caller and the callee each run as the AST and as byte
# code. GNU R's default JIT (R_CheckJIT's TOP_SMALL_MAYBE) runs a small
# top-level closure's first call as the AST and compiles it at its second;
# the top level itself is the AST.
bt <- function(s, e) .Internal(getBindingType(s, e))
y <- 2
f <- function(a, b, cc, d = 1) {
  env <- environment()
  r <- c(bt(quote(a), env), bt(quote(b), env), bt(quote(cc), env), bt(quote(d), env))
  a; b; cc; d
  c(r, "|", bt(quote(a), env), bt(quote(b), env), bt(quote(cc), env), bt(quote(d), env))
}
# AST caller: f's first call (AST), then past its threshold (byte code)
print(f(1, y, y + 1))
print(f(1, y, y + 1))
print(f(1, y, y + 1))
h <- function(a, b, cc, d = 1) {
  env <- environment()
  r <- c(bt(quote(a), env), bt(quote(b), env), bt(quote(cc), env), bt(quote(d), env))
  a; b; cc; d
  c(r, "|", bt(quote(a), env), bt(quote(b), env), bt(quote(cc), env), bt(quote(d), env))
}
# a caller compiled before its callee's first call: byte code calling the
# AST, then byte code calling byte code
k <- function(go) if (go) h(1, y, y + 1) else "first call"
print(k(FALSE))
print(k(TRUE))
print(k(TRUE))
print(k(TRUE))
# a caller's first call (AST) calling a compiled callee, then compiled
g <- function() f(1, y, y + 1)
print(g())
print(g())
# the same arguments through a defaulted argument that is supplied
print(f(1, y, y + 1, 5))
print(k(TRUE))
