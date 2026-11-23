# INTERIM(C1) (recorded, not ratcheted; a port, owned by C1's rebuilt PR A
# on PR B, #1029): findFunDef's get() runs an active binding of the called
# name at the compile, the notes suppressed or not; robigo-rt's fun_def
# does not run it.
invisible(compiler::enableJIT(2))
makeActiveBinding("myfun", function() {
    cat("active\n")
    function() 9
}, .GlobalEnv)
f <- function(x) { if (x) myfun(); 7 }
print(f(FALSE))
print(f(FALSE))
