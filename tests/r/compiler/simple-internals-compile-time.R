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
