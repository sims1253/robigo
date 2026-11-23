# INTERIM(C1) (recorded, not ratcheted; a port, owned by C1's rebuilt PR A
# on PR B, #1029): a function literal that may call browser() is declined
# by the `function` handler (mayCallBrowser), and cmpCall then compiles the
# call as a call: findFunDef's check and the arguments' code, the
# literal's body included, where `myfun`'s lazy binding is forced.
invisible(compiler::enableJIT(2))
delayedAssign("myfun", { cat("forced\n"); function() 9 })
f <- function(x) {
    if (x) identity(function() { browser(); myfun() })
    7
}
print(f(FALSE))
print(f(FALSE))
