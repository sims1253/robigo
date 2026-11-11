# Where GNU R's compiler inlines the simple .Internal wrappers
# (cmp.R:safeBaseInternals: nchar, identical, rep.int, ...): only in
# bytecode. A top-level loop is compiled (eval.c:do_for at JIT level 3,
# BASEGUARD-checked); so are a warmed closure's body and the promises its
# code makes, wherever they are forced. The AST evaluator calls the
# closure: plain top-level calls, a closure's first call, local().
# The wrapper's frame shows in sys.calls() from a calling handler.
heads <- function() {
    s <- sys.calls()
    s <- s[seq_len(length(s) - 2L)]
    cat(vapply(s, function(x) paste(deparse(x[[1L]]), collapse = ""), ""), "\n")
}
err <- function(e) { heads(); invokeRestart("skip") }
probe <- function(expr) withRestarts(expr, skip = function() NULL)

cat("-- top level\n")
probe(withCallingHandlers(nchar(factor("a")), error = err))
cat("-- top-level loops\n")
for (i in 1) probe(withCallingHandlers(nchar(factor("a")), error = err))
i <- 0
while (i < 1) { i <- i + 1; probe(withCallingHandlers(nchar(factor("a")), error = err)) }
repeat { probe(withCallingHandlers(rep.int(1, -1), error = err)); break }
cat("-- local()\n")
local(for (i in 1) probe(withCallingHandlers(nchar(factor("a")), error = err)))
cat("-- closure promise, first and warmed calls\n")
g <- function(x) probe(withCallingHandlers(nchar(x), error = err))
for (k in 1:3) g(factor("a"))
cat("-- dots stay generic\n")
d <- function(...) nchar(...)
for (i in 1) probe(withCallingHandlers(d(factor("a")), error = err))

cat("-- error and warning calls in a top-level loop\n")
for (i in 1) {
    print(tryCatch(nchar("a", type = "zz"), error = conditionCall))
    print(tryCatch(nchar(type = "zz", x = "a"), error = conditionMessage))
    print(tryCatch(nchar(factor("a")), error = conditionCall))
    print(tryCatch(rep.int(1:2, -1), error = conditionCall))
    print(tryCatch(identical(), error = conditionMessage))
    print(tryCatch(nchar("a", "chars", FALSE, NA, 1), error = conditionMessage))
    print(nchar(c("abc", NA), keepNA = FALSE))
    print(nchar(type = "bytes", "é"))
    print(match("b", letters))
    print(withCallingHandlers(nchar({ heads(); "abc" }), error = err))
}

cat("-- a shadowing definition is called\n")
nchar <- function(x, ...) { cat("user nchar\n"); 0L }
for (i in 1:2) print(nchar("abc"))
f <- function(v) nchar(v)
for (k in 1:3) print(f("abc"))
rm(nchar)
for (k in 1:2) print(f("abc"))
for (i in 1:2) {
    if (i == 2) nchar <- function(x, ...) { cat("rebound in the loop\n"); 0L }
    print(nchar("abc"))
}
rm(nchar)

cat("-- trace()\n")
invisible(trace("nchar", quote(cat("traced\n")), print = FALSE))
for (i in 1) print(nchar("abc"))
invisible(untrace("nchar"))
for (i in 1) print(nchar("abc"))

cat("-- a top-level loop compiles at each run\n")
ex <- quote(for (i in 1) print(identical(1, 1)))
eval(ex)
orig <- baseenv()$identical
unlockBinding("identical", baseenv())
assign("identical", function(x, y, ...) { cat("new identical\n"); TRUE }, baseenv())
eval(ex)
assign("identical", orig, baseenv())
lockBinding("identical", baseenv())
eval(ex)
