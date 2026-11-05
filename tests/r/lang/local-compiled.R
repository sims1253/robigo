# GNU R's byte compiler inlines `local(expr)` (one argument, `local` being
# base's) as the call `(function() expr)()` with a compiled body
# (compiler/R/cmp.R: setInlineHandler("local")). Compiled code is what the
# JIT compiled (a closure at its first call when large, its second when
# small and at top level) and installed packages; elsewhere base's local()
# evaluates `expr` with eval() in a new environment.
probe <- function() {
  outer <- environment()
  list(
    call = deparse(local(sys.call())),
    fun = deparse(local(sys.function())),
    parent_is_outer = local(identical(parent.frame(), outer)),
    env_parent_is_outer = local(identical(parent.env(environment()), outer)),
    nframe = local(sys.nframe()) - sys.nframe(),
    ret = local(return(5)),
    leaks = { local(zz <- 1); exists("zz", inherits = FALSE) },
    with_envir = deparse(local(sys.call(), envir = new.env()))
  )
}
show <- function(x) for (n in names(x)) cat(" ", n, ":", paste(format(x[[n]]), collapse = " "), "\n")
big <- function() {
  a <- 1; b <- 2; c <- 3; dd <- 4; e <- 5; g <- 6; h <- 7; k <- 8
  a <- a + b + c + dd + e + g + h + k; b <- a * 2; c <- b - a; dd <- c * c
  e <- dd + a; g <- e - b; h <- g + c; k <- h + dd
  probe()
}
# probe is small: interpreted at its first call, compiled at its second
show(probe())
show(probe())
# called from a compiled closure, probe's own state decides
show(big())
# at top level and in eval(), base's local() runs
cat(deparse(local(sys.call())), "\n")
print(local(identical(parent.frame(), globalenv())))
# a closure defined inside a compiled closure is compiled with it
mk <- function() {
  a <- 1; b <- 2; c <- 3; dd <- 4; e <- 5; g <- 6; h <- 7; k <- 8
  a <- a + b + c + dd + e + g + h + k; b <- a * 2; c <- b - a; dd <- c * c
  e <- dd + a; g <- e - b; h <- g + c; k <- h + dd
  function() deparse(local(sys.call()))
}
inner <- mk()
cat(inner(), "\n")
# a top-level loop is compiled
for (i in 1) cat(deparse(local(sys.call())), "\n")
