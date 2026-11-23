# INTERIM(C1) (recorded, not ratcheted; order, owned by PR B, #1029):
# getInlineInfo compares compilerOptions' optimize at each of its calls,
# so an Ops method of a classed optimize runs at each (three times here);
# robigo-rt makes the comparisons once, at the entry's read
# (compile/options.rs `level_in_r`).
invisible(compiler::enableJIT(2))
o <- compiler:::compilerOptions
n <- 0L
Ops.level <- function(e1, e2) {
    if (.Generic == ">") {
        n <<- n + 1L
        return(n != 2L)
    }
    TRUE
}
assign("optimize", structure(2, class = "level"), envir = o)
f <- function() 7
print(f())
print(f())
print(n)
assign("optimize", 2, envir = o)
