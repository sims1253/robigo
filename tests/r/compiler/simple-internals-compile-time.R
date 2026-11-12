# GNU R decides a simple .Internal wrapper's inline when it compiles the
# code (cmp.R:getInlineInfo), and guards it at every run (BASEGUARD):
# - a promise compiled code made is bytecode even after its creator
#   returned;
# - a top-level loop is compiled when it starts (eval.c:do_for), against
#   the definitions then, not when a call is first reached.
heads <- function() {
    s <- sys.calls()
    cat(vapply(s, function(x) paste(deparse(x[[1L]]), collapse = ""), ""), "\n")
}

cat("-- a promise forced after its creator returned\n")
defer <- function(x) function() x
maker <- function() defer(nchar({ heads(); "abc" }))
for (k in 1:3) { p <- maker(); print(p()) }
maker2 <- function() { q <- defer(nchar({ heads(); "abc" })); q() }
for (k in 1:3) print(maker2())
bad <- function() defer(nchar(factor("a")))
for (k in 1:3) { p <- bad(); print(tryCatch(p(), error = conditionCall)) }
p <- maker()
nchar <- function(x, ...) { cat("user nchar\n"); 0L }
print(p())
rm(nchar)

cat("-- a top-level loop resolves when it starts\n")
orig <- baseenv()$nchar
unlockBinding("nchar", baseenv())
for (i in 1:2) {
    if (i == 1) assign("nchar", function(x) 42L, baseenv())
    print(nchar("abc"))
}
assign("nchar", orig, baseenv())
lockBinding("nchar", baseenv())
nchar <- function(x, ...) { cat("user nchar\n"); 0L }
for (i in 1:2) {
    if (i == 1) rm(nchar)
    print(nchar({ heads(); "abc" }))
}
for (i in 1:3) {
    if (i == 1) nchar <- function(x, ...) { cat("user nchar\n"); 0L }
    if (i == 2) rm(nchar)
    print(nchar({ heads(); "abc" }))
}
for (i in 1:2) for (j in 1:2) print(nchar({ heads(); "ab" }))

cat("-- names the code assigns are locals (cmp.R:findLocals)\n")
f <- function(k) { if (k > 5) nchar <- function(x) 0L; nchar({ heads(); "abc" }) }
for (k in 1:3) print(f(k))
g <- function(k) { if (k > 5) assign("identical", 1); identical({ heads(); 1 }, 1) }
for (k in 1:3) print(g(k))
h <- function(nchar = 1) nchar({ heads(); "abc" })
for (k in 1:3) print(h())
q <- function() { e <- quote(nchar <- 1); nchar({ heads(); "abc" }) }
for (k in 1:3) print(q())
w <- function() { e <- function() nchar <- 1; nchar({ heads(); "abc" }) }
for (k in 1:3) print(w())
for (i in 1:2) { if (i > 5) nchar <- 1; print(nchar({ heads(); "abc" })) }
for (i in 1:2) { if (i > 5) for (nchar in 1) 0; print(nchar({ heads(); "abc" })) }

cat("-- a binding of any type is found at compile time\n")
nchar <- 1
for (i in 1:2) print(nchar({ heads(); "abc" }))
v <- function() nchar({ heads(); "abc" })
for (k in 1:3) print(v())
rm(nchar)

cat("-- code below the base namespace finds base there\n")
u <- function() { for (j in 1) r <- nchar({ heads(); "abc" }); r }
environment(u) <- new.env(parent = .BaseNamespaceEnv)
for (k in 1:3) print(u())

cat("-- a closure decides when the JIT compiles it, before its body runs\n")
f2 <- function() { rm("nchar", envir = .GlobalEnv); nchar({ heads(); "abc" }) }
for (k in 1:3) {
    assign("nchar", function(x, ...) { cat("user nchar\n"); 0L }, .GlobalEnv)
    print(f2())
}
f3 <- function(a) if (a) nchar({ heads(); "abc" }) else 0L
for (k in 1:2) print(f3(FALSE))
nchar <- function(x, ...) { cat("user nchar\n"); 0L }
print(f3(TRUE))
rm(nchar)
print(f3(TRUE))

cat("-- a closure compiled code makes is compiled with its creator\n")
nchar <- function(x, ...) { cat("user nchar\n"); 0L }
mk <- function() function() nchar({ heads(); "abc" })
for (k in 1:2) g1 <- mk()
rm(nchar)
for (k in 1:2) print(g1())
mk2 <- function() function() { if (FALSE) nchar <- 1; nchar({ heads(); "abc" }) }
for (k in 1:2) g2 <- mk2()
for (k in 1:2) print(g2())

cat("-- a detached promise keeps its creator's compile-time locals\n")
maker3 <- function() { if (FALSE) nchar <- 1; defer(nchar({ heads(); "abc" })) }
for (k in 1:3) { p <- maker3(); print(p()) }
nchar <- function(x, ...) { cat("user nchar\n"); 0L }
maker4 <- function() defer(nchar({ heads(); "abc" }))
for (k in 1:2) p <- maker4()
rm(nchar)
for (k in 1:2) { p <- maker4(); print(p()) }

cat("-- compiled code keeps the definition it was compiled against\n")
v3 <- function(a) if (a) nchar("abc") else 0L
for (k in 1:2) print(v3(FALSE))
orig <- baseenv()$nchar
unlockBinding("nchar", baseenv())
assign("nchar", function(x, type = "chars", allowNA = FALSE, keepNA = NA) 42L, baseenv())
print(v3(TRUE))
v4 <- function(a) if (a) nchar("abc") else 0L
for (k in 1:2) print(v4(FALSE))
print(v4(TRUE))
assign("nchar", orig, baseenv())
lockBinding("nchar", baseenv())
print(v3(TRUE))
print(v4(TRUE))

cat("-- a nested closure's locals are found when its creator compiles\n")
mk <- function() function() { local(nchar <- 1); nchar({ heads(); "abcf" }) }
g1 <- mk(); g1 <- mk()
local <- function(expr) eval(substitute(expr), new.env())
print(g1())
rm(local)

cat("-- wrapper calls inside an inlined local() body\n")
lf <- function() local(nchar({ heads(); "abcg" }))
lf(); lf(); lf()

cat("-- the JIT cache does not match a hashed environment\n")
hf <- function() { for (i in 1) {}; nchar({ heads(); "abch" }) }
hf()
hg <- function() { for (i in 1) {}; nchar({ heads(); "abch" }) }
environment(hg) <- new.env(hash = TRUE, parent = .GlobalEnv)
hg(); hg(); hg()

cat("-- the JIT cache compares the closure's srcref when the body has none\n")
options(keep.source = TRUE)
eval(parse(text = 'sf <- function(a) if (a) nchar("\\u00e9") else 0L', keep.source = TRUE))
sf(FALSE); sf(FALSE)
orig <- baseenv()$nchar
unlockBinding("nchar", baseenv())
assign("nchar", function(x, type = "chars", allowNA = FALSE, keepNA = NA) .Internal(nchar(x, "bytes", allowNA, keepNA)), baseenv())
eval(parse(text = 'sg <- function(a) if (a) nchar("\\u00e9") else 0L', keep.source = TRUE))
sg(FALSE); sg(FALSE)
print(sg(TRUE))
print(sf(TRUE))
assign("nchar", orig, baseenv())
lockBinding("nchar", baseenv())
options(keep.source = FALSE)

cat("-- a lazy wrapper binding is forced when the code compiles\n")
unlockBinding("nchar", baseenv())
delayedAssign("nchar", { cat("forced wrapper\n"); orig }, assign.env = baseenv())
zf <- function(a) if (a) nchar("abcz") else 0L
cat("first\n"); print(zf(FALSE))
cat("second\n"); print(zf(FALSE))
cat("third\n"); print(zf(TRUE))
assign("nchar", orig, baseenv())
lockBinding("nchar", baseenv())

cat("-- a closure made in an assign() argument is compiled with its creator\n")
maker5 <- function() { assign("child5", function() nchar({ heads(); "abci" }), envir = .GlobalEnv); 0L }
maker5(); maker5(); print(child5())

cat("-- the JIT cache slot is GNU R's hash of the body\n")
cf <- function(a) if (a) nchar("abcq") else 0L
cf(FALSE); cf(FALSE)
cfill <- function(a) if (a) nchar("abcq") else 1024L
cfill(FALSE); cfill(FALSE)
unlockBinding("nchar", baseenv())
assign("nchar", function(x, type = "chars", allowNA = FALSE, keepNA = NA) 42L, baseenv())
cg <- function(a) if (a) nchar("abcq") else 0L
cg(FALSE); cg(FALSE)
print(cg(TRUE))
assign("nchar", orig, baseenv())
lockBinding("nchar", baseenv())

cat("-- the arguments of a special the compiler inlines are constants\n")
lazy <- function() {
    unlockBinding("nchar", baseenv())
    delayedAssign("nchar", { cat("forced wrapper\n"); orig }, assign.env = baseenv())
}
lazy()
tf <- function(a) if (a) ~nchar("abct") else 0L
cat("tilde\n"); invisible(tf(FALSE)); invisible(tf(FALSE))
lazy()
ef <- function(a) if (a) expression(nchar("abce")) else 0L
cat("expression\n"); invisible(ef(FALSE)); invisible(ef(FALSE))
lazy()
qf <- function(a) if (a) quote(nchar("abcu")) else 0L
cat("quote\n"); invisible(qf(FALSE)); invisible(qf(FALSE))
lazy()
rf <- function(a) if (a) rep(nchar("abcr"), 2L) else 0L
cat("rep\n"); invisible(rf(FALSE)); invisible(rf(FALSE))
lazy()
lf <- function(a) if (FALSE) nchar("abcl") else a
cat("if (FALSE)\n"); invisible(lf(1L)); invisible(lf(1L))
lazy()
sf <- function(a) { call <- 1; if (a) call("f", nchar("abcs")) else 0L }
cat("a local named call\n"); invisible(sf(FALSE)); invisible(sf(FALSE))
assign("nchar", orig, baseenv())
lockBinding("nchar", baseenv())
