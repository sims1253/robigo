# GNU R byte-compiles a top-level loop, so a builtin's warning raised in one
# names its call ("In as.double(z) :"); the same call outside a loop does
# not. Found by scripts/fuzz-eval.py --gen template, reduced by
# scripts/reduce.py.
z <- 1i
for (i in 1:2) x <- as.double(z)
x <- as.double(z)
repeat {
    y <- as.integer(z)
    break
}
i <- 0L
while (i < 1L) {
    i <- i + 1L
    w <- as.double(z)
}
