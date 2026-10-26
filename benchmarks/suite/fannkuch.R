# fannkuch-redux (Benchmarks Game): integer vectors, indexing, while loops.
fannkuch <- function(n) {
  perm1 <- 0:(n - 1); count <- integer(n); maxflips <- 0L; checksum <- 0L; permcount <- 0L
  r <- n
  repeat {
    while (r != 1) { count[r] <- r; r <- r - 1L }
    perm <- perm1; flips <- 0L
    k <- perm[1]
    while (k != 0) {
      perm[1:(k + 1)] <- perm[(k + 1):1]
      flips <- flips + 1L
      k <- perm[1]
    }
    if (flips > maxflips) maxflips <- flips
    checksum <- checksum + if (permcount %% 2L == 0L) flips else -flips
    repeat {
      if (r == n) return(c(checksum, maxflips))
      p0 <- perm1[1]
      for (i in 1:r) perm1[i] <- perm1[i + 1]
      perm1[r + 1] <- p0
      count[r + 1] <- count[r + 1] - 1L
      if (count[r + 1] > 0) break
      r <- r + 1L
    }
    permcount <- permcount + 1L
  }
}
cat(fannkuch(8L), "\n")
