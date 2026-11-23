# A function literal whose funEnv warns (a name, written as an escape so
# that it is UTF-8 marked, that does not translate under LC_ALL=C), a
# calling handler that makes the active compilerOptions binding plain: the
# literal's warnings come before its reads (make.functionContext's funEnv,
# then its make.toplevelContext), so after the handler there is nothing to
# read. robigo-rt raises the warnings a compile gave before a function
# context's reads there.
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
f <- function() { for (i in 1:2) NULL; g <- function() { "\u4e2d" <- 1 }; 7 }
withCallingHandlers(print(f()), warning = function(w) {
    cat("handler\n")
    if (bindingIsActive("optimize", o)) { rm("optimize", envir = o); assign("optimize", 2, envir = o) }
    invokeRestart("muffleWarning")
})
