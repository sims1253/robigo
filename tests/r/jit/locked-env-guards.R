# Closures whose environment is a locked chain up to the search path (as
# namespace code is): guarded base functions in their frames, rebinding
# through the locked environment, and frames that bind the name.
ns <- new.env(parent = baseenv())
f <- function(x) { s <- 0; for (i in seq_along(x)) s <- s + length(x[[i]]); s }
g <- function(x) length(x) * 2L
environment(f) <- ns; environment(g) <- ns
assign("f", f, ns); assign("g", g, ns)
lockEnvironment(ns, bindings = TRUE)
lst <- list(1:3, 1:2, 4)
for (k in 1:3) print(c(f(lst), g(lst)))
# A function bound in the locked environment (after unlocking its binding
# is impossible for a new name; replace an existing one).
unlockBinding("g", ns)
assign("g", function(x) { cat("new g\n"); 0L }, ns)
print(f(lst)); print(ns$g(lst))
# Rebinding a guarded name where the frame's enclosure is locked: only
# through the frame itself.
h <- function(x) { if (x > 1) length <- function(y) 99L; length(1:3) }
environment(h) <- ns
for (x in c(1, 2, 1)) print(h(x))
# A namespace's own closures, called repeatedly.
for (k in 1:3) print(stats::median(c(5, 3, 1, k)))
print(utils::head(letters, 3))
