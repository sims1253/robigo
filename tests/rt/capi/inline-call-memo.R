# A closure called from an inlined body: the site remembers the code it ran
# inline, and what it remembered must not outlive the callee it judged
# (docs/g3.md §9.3). Each change below lands between two calls of the same
# site, and the next call runs what GNU R runs: a redefined callee, a
# default written from C, a formal renamed from C (GNU R reads a closure's
# formals at every call).
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/closure_mutation.so"))
sq <- function(x, k = 2) x^k
h <- function(i) sq(i)
g <- function(n, change) {
  s <- numeric(n)
  for (i in 1:n) {
    s[i] <- h(i)
    if (i == 3) change()
  }
  s
}
print(g(6, function() NULL))
# the callee redefined
print(g(6, function() sq <<- function(x, k = 2) x + 100))
sq <- function(x, k = 2) x^k
# its default written from C
print(g(6, function() invisible(.Call("rt_set_formal_default", sq, 1L, 3))))
sq <- function(x, k = 2) x^k
# its formal renamed from C: a call that names it no longer matches
h <- function(i) sq(i, k = 3)
print(tryCatch(g(6, function() invisible(.Call("rt_set_formal_tag", sq, 1L, quote(j)))),
  error = conditionMessage))
