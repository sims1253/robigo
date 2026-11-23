# tryCmpfun (#964): compilerOptions$suppressAll read as cmp.R's
# notifyCompilerError reads it, at each failed compile: flipped between
# compiles, a classed value. A value `!` or `if`
# rejects, or the binding gone, is an error of the handler: it leaves the
# call and leaves the JIT off (GNU R restores R_jit_enabled only when the
# compile returns), which enableJIT shows and turns back on. Run under
# LC_ALL=C.
options(warn = 2)
o <- compiler:::compilerOptions
compiler:::setCompilerOptions(suppressAll = FALSE)
f1 <- function() { "\u4e2d" <- 1L; for (i in 1) NULL; 0 }
r <- try(f1(), silent = TRUE); cat('FALSE', "->", r)
compiler:::setCompilerOptions(suppressAll = TRUE)
f2 <- function() { "\u4e2d" <- 2L; for (i in 1) NULL; 0 }
r <- try(f2(), silent = TRUE); cat('TRUE', "->", r)
compiler:::setCompilerOptions(suppressAll = FALSE)
f3 <- function() { "\u4e2d" <- 3L; for (i in 1) NULL; 0 }
r <- try(f3(), silent = TRUE); cat('FALSE again', "->", r)
assign("suppressAll", structure(TRUE, class = "foo"), envir = o)
f4 <- function() { "\u4e2d" <- 4L; for (i in 1) NULL; 0 }
r <- try(f4(), silent = TRUE); cat('classed TRUE', "->", r)
`!.foo` <- function(x) FALSE
assign("suppressAll", structure(FALSE, class = "foo"), envir = o)
f5 <- function() { "\u4e2d" <- 5L; for (i in 1) NULL; 0 }
r <- try(f5(), silent = TRUE); cat('classed FALSE, a ! method', "->", r)
assign("suppressAll", 1L, envir = o)
f6 <- function() { "\u4e2d" <- 6L; for (i in 1) NULL; 0 }
r <- try(f6(), silent = TRUE); cat('1L', "->", r)
assign("suppressAll", NA, envir = o)
f7 <- function() { "\u4e2d" <- 7L; for (i in 1) NULL; 0 }
r <- try(f7(), silent = TRUE); cat('NA', "->", r)
cat("JIT level after:", compiler::enableJIT(3L), "\n")
assign("suppressAll", "yes", envir = o)
f8 <- function() { "\u4e2d" <- 8L; for (i in 1) NULL; 0 }
r <- try(f8(), silent = TRUE); cat('"yes"', "->", r)
cat("JIT level after:", compiler::enableJIT(3L), "\n")
assign("suppressAll", c(TRUE, FALSE), envir = o)
f9 <- function() { "\u4e2d" <- 9L; for (i in 1) NULL; 0 }
r <- try(f9(), silent = TRUE); cat('c(TRUE, FALSE)', "->", r)
cat("JIT level after:", compiler::enableJIT(3L), "\n")
assign("suppressAll", logical(0), envir = o)
f10 <- function() { "\u4e2d" <- 10L; for (i in 1) NULL; 0 }
r <- try(f10(), silent = TRUE); cat('logical(0)', "->", r)
cat("JIT level after:", compiler::enableJIT(3L), "\n")
rm("suppressAll", envir = o)
f12 <- function() { "\u4e2d" <- 12L; for (i in 1) NULL; 0 }
r <- try(f12(), silent = TRUE); cat('removed', "->", r)
cat("JIT level after:", compiler::enableJIT(3L), "\n")
