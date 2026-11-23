# A function literal whose funEnv warns (a name that does not translate,
# LC_ALL=C), a calling handler that makes the active compilerOptions
# binding plain: the reads as GNU R's.
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
f <- function() { for (i in 1:2) NULL; g <- function() { "中" <- 1 }; 7 }
withCallingHandlers(print(f()), warning = function(w) {
    if (bindingIsActive("optimize", o)) { rm("optimize", envir = o); assign("optimize", 2, envir = o) }
    invokeRestart("muffleWarning")
})
