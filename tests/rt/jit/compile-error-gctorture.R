# tryCmpfun (#964): a compile an error ends under gctorture, then
# a compile that succeeds. Run under LC_ALL=C.
f <- function() { "\u4e2d" <- 1; for (i in 1) NULL; 0 }
g <- function(x) { s <- 0; for (i in seq_len(x)) s <- s + i; s }
options(warn = 2)
gctorture(TRUE)
r <- try(f(), silent = TRUE)
x <- g(3L)
gctorture(FALSE)
cat(r)
print(x)
