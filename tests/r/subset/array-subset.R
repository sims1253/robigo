# Adapted from GNU R's tests/array-subset.R (matrix() and array() replaced).
mk <- function(data, dim, dimnames = NULL, byrow = FALSE) {
  if (byrow) {
    nr <- dim[1]; nc <- dim[2]
    out <- data
    for (i in 1:nr) for (j in 1:nc) out[i + (j - 1) * nr] <- data[(i - 1) * nc + j]
    data <- out
  }
  dim(data) <- dim
  if (!is.null(dimnames)) dimnames(data) <- dimnames
  data
}
fails <- function(expr) tryCatch({ expr; FALSE }, error = function(e) TRUE)
m <- mk(1:25, c(5L, 5L), dimnames = list(letters[1:5], LETTERS[1:5]))
si <- mk(c(1, 1, 2, 3, 3, 4), c(3L, 2L), byrow = TRUE)
ss <- mk(c("a", "A", "b", "C", "c", "D"), c(3L, 2L), byrow = TRUE)
identical(m[si], m[ss])
identical(c(1L, 12L, 18L), m[ss])
ssna <- ss; ssna[2, 2] <- NA
identical(c(1L, NA, 18L), m[ssna])
ssna <- ss; ssna[2, 1] <- NA
identical(c(1L, NA, 18L), m[ssna])
mnadim <- m
tmp <- dimnames(mnadim)
tmp[[1]][5] <- NA
dimnames(mnadim) <- tmp
identical(c(1L, NA, 18L), mnadim[ssna])
ssnm <- ss
ssnm[2, 2] <- "NOMATCH"
fails(m[ssnm])
mnadim <- m
tmp <- dimnames(mnadim)
tmp[[1]][5] <- ""
dimnames(mnadim) <- tmp
ssnm <- ss
ssnm[2, 2] <- ""
fails(mnadim[ssnm])
m3 <- m2 <- m
m2[si] <- c(100L, 200L, 300L)
m3[ss] <- c(100L, 200L, 300L)
identical(m2, m3)
m2
a <- mk(1:75, c(5L, 5L, 3L), dimnames = list(letters[1:5], LETTERS[1:5], letters[24:26]))
si <- mk(c(1, 1, 1, 2, 3, 1, 3, 4, 1, 5, 1, 3), c(4L, 3L), byrow = TRUE)
ss <- mk(c("a", "A", "x", "b", "C", "x", "c", "D", "x", "e", "A", "z"), c(4L, 3L), byrow = TRUE)
identical(a[si], a[ss])
identical(c(1L, 12L, 18L, 55L), a[ss])
a2 <- a1 <- a
a1[si] <- c(100L, 1200L, 1800L, 5500L)
a2[ss] <- c(100L, 1200L, 1800L, 5500L)
identical(a1, a2)
a3 <- a
dn <- dimnames(a3)
dn[2] <- list(NULL)
dimnames(a3) <- dn
fails(a3[ss])
tryCatch(a3[ss], error = function(e) conditionMessage(e))
