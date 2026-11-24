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
# the formals writes above made every closure's code stale; cb's byte code
# is still GNU R's: its written body does not run
print(cb()); print(cb())
# a quoted constant that C made cyclic: the code quotes it, nothing walks
# it (a body GNU R never compiles: its JIT decides so at the second call,
# before the write, and then runs the AST)
qf <- function() { if (FALSE) browser(); quote(x) }
print(qf()); print(qf())
invisible(.Call("rt_set_quote_cycle", qf, 2L))
print(typeof(qf())); print(typeof(qf()))
# a quoted call made code by a new head (`{`), then written: the AST runs
# the written call
qq <- function() { if (FALSE) browser(); quote(1 + 2) }
print(qq()); print(qq())
invisible(.Call("rt_set_call_head", qq, 2L, quote(`{`)))
print(qq())
invisible(.Call("rt_set_arg_elt", qq, 2L, 2L, 100))
print(qq())
# a call's names removed from C (setAttrib to NULL): the AST matches the
# arguments by position from then on
hm <- function(a, b) a - b
sw <- function() { if (FALSE) browser(); hm(b = 2, a = 1) }
print(sw()); print(sw())
invisible(.Call("rt_unname_call", sw, 2L))
print(sw())
# a call whose arguments C made an object's attribute cells: setAttrib on
# the object from C writes the call's arguments (a new attribute appends
# one, removing it strips it, a new value replaces one); the code matches
# the arguments as they are, by name
hm3 <- function(a, b, c) a - b + 10 * c
ax <- structure(0, a = 1, b = 2, c = 3)
sa <- function() { if (FALSE) browser(); hm3(0, 0, 0) }
print(sa()); print(sa())
invisible(.Call("rt_share_attrib", sa, 2L, ax))
print(sa()); print(sa())
# b stripped, then appended after c: (a, c, b), as many arguments as before
invisible(.Call("rt_set_attr", ax, "b", NULL))
invisible(.Call("rt_set_attr", ax, "b", 2))
print(sa())
invisible(.Call("rt_set_attr", ax, "d", 5))
print(tryCatch(sa(), error = conditionMessage))
invisible(.Call("rt_set_attr", ax, "a", 4))
invisible(.Call("rt_set_attr", ax, "d", NULL))
print(sa())
# a closure compiled by cmpfun: its BODY is byte code, so R_ClosureExpr is
# the byte code's expression (eval.c bytecodeExpr); a write into it, after
# the first call, shows in body() but does not run
cf <- compiler::cmpfun(function() { 1; "old" })
print(cf())
print(.Call("rt_body", cf))
invisible(.Call("rt_set_body_elt", cf, 2L, "new"))
print(cf()); print(cf())
print(.Call("rt_body", cf))
print(body(cf))
# two closures `function` made from one compiled body share it (and its
# formals): C's writes reach both, and both keep running their byte code
# (and robigo-rt's one translation of it)
fac <- compiler::cmpfun(function(a) function(x = 1) { a; "old" })
p1 <- fac(1); p2 <- fac(2)
print(c(p1(), p2()))
invisible(.Call("rt_set_body_elt", p1, 2L, "new"))
print(c(p1(), p2()))
print(identical(body(p1), body(p2)))
invisible(.Call("rt_set_formal_default", p2, 0L, 9))
print(c(formals(p1)$x, formals(p2)$x))
