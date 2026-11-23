# A fold with an S3 method that prints, before two function literals whose
# compilerOptions reads stop the compile: the fold is evaluated once, its
# value kept across the reads (cmp.R folds once).
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
`+.cls` <- function(e1, e2) { cat("fold\n"); TRUE }
VAL <- structure(1, class = "cls")
f <- function() {
    for (i in 1:2) NULL
    if (VAL + 1) NULL
    g <- function() NULL
    h <- function() NULL
    3
}
print(f())
