# Package C writing a closure's body and formals in place (docs/g3.md
# §9.3, §15.3 E6): what runs next is what GNU R runs. GNU R walks an AST
# body live, so the next call sees the write; byte code (a closure past
# GNU R's JIT threshold) is a separate object, so a write into the
# expression R_ClosureExpr returns changes what it (and `body(f)`)
# return but not what runs;
# formals are read at every call, AST or byte code: the next call matches
# the new names and promises the new defaults.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/closure_mutation.so"))
# a body GNU R never compiles (it may call browser()): always the AST
nb <- function() { if (FALSE) browser(); "old" }
print(nb()); print(nb()); print(nb())
invisible(.Call("rt_set_body_elt", nb, 2L, quote(paste("new", "body"))))
print(nb()); print(nb())
print(.Call("rt_body", nb))
# a small top-level closure written between its first call (the AST) and
# its second (which compiles it): the compiled code is the new body's
s <- function() { 1; "old" }
print(s())
invisible(.Call("rt_set_body_elt", s, 2L, "new"))
print(s()); print(s())
# a closure past its threshold: its byte code stays, R_ClosureExpr shows
# the write
cb <- function() { 1; "old" }
print(cb()); print(cb()); print(cb())
invisible(.Call("rt_set_body_elt", cb, 2L, "new"))
print(cb()); print(cb())
print(.Call("rt_body", cb))
# a default replaced: both the AST and byte code promise the new one
d <- function(x = 1) x
print(d()); print(d()); print(d())
invisible(.Call("rt_set_formal_default", d, 0L, quote(1 + 1)))
print(d()); print(d())
# a default call written in place: its promise evaluates the new call
y <- 1
k <- function(x = y + 1) x
print(k()); print(k()); print(k())
invisible(.Call("rt_set_default_elt", k, 0L, 2L, 100))
print(k()); print(k())
print(.Call("rt_formals", k)[["x"]])
# a formal renamed: the next call matches the new name, and the body's
# reference to the old one is a free variable
a <- "global a"
h <- function(a = 10) a
print(h()); print(h()); print(h(a = 3))
invisible(.Call("rt_set_formal_tag", h, 0L, quote(b)))
print(h()); print(h(b = 3))
print(tryCatch(h(a = 3), error = conditionMessage))
print(names(.Call("rt_formals", h)))
# the same writes on a closure whose first call has not run
fresh <- function(x = 1) { 1; x }
invisible(.Call("rt_set_formal_default", fresh, 0L, 7))
invisible(.Call("rt_set_body_elt", fresh, 1L, quote(x <- x + 1)))
print(fresh()); print(fresh()); print(fresh())
