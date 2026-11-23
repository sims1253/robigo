# Results like TRUE or length(x) may be shared constants; modifying a copy
# must never change them
x <- TRUE; x[2] <- FALSE; x
y <- TRUE; y
attr(x, "a") <- 1; x
z <- (1 == 1); attributes(z); z
f <- function() { v <- length(1:5); v[1] <- 99L; v }
f(); length(1:5)
g <- function() { w <- is.na(NA); names(w) <- "n"; w }
g(); is.na(NA)
h <- function() { k <- 3L; k[3] <- 7L; k }
h(); 3L
n <- 0L
for (i in 1:5) { m <- 5L; m[1] <- m[1] + i; n <- n + m }
n; 5L
l <- list(TRUE, 1L); l[[1]][2] <- NA; l; TRUE
s <- sum(c(TRUE, TRUE)); s[1] <- 10L; s; sum(c(TRUE, TRUE))
dim(x) <- c(1, 2); x; TRUE
