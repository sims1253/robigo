# Closures created by compiled code are bytecode; formal defaults never are
# (formals stay AST even in installed bytecode), and `environment<-` switches
# a compiled body back to its interpreted version (builtin.c:do_envirgets).
show <- function(f) {
    path <- tempfile()
    sink(path)
    print(f)
    sink()
    lines <- readLines(path)
    unlink(path)
    cat(gsub("0x[0-9a-f]+", "0x", lines), sep = "\n")
    cat("\n")
}

# A namespace function's body closures are compiled with it (MAKECLOSURE).
show(stats::power(1/3)$linkfun)
# ... while closures from formal defaults stay interpreted.
show(utils::strOptions()$formatNum)
# make.link assigns the namespace as environment: the decompiled closures
# print without bytecode again.
show(stats::Gamma()$linkinv)
# A user closure compiled at its first call (a loop, so JIT score >= 50)
# creates compiled closures too ...
h <- function(lam) { for (i in 1:60) NULL; n <- 0; function(mu) { n <<- n + 1; mu^lam + n } }
k <- h(2)
show(k)
# ... until `environment<-` switches k back to its interpreted body.
environment(k) <- baseenv()
show(k)
