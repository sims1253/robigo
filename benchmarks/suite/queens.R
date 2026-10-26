# n-queens by backtracking: recursion, logical vectors.
queens <- function(n) {
  cols <- logical(n); d1 <- logical(2 * n); d2 <- logical(2 * n)
  place <- function(r) {
    if (r > n) return(1)
    s <- 0
    for (c in 1:n) {
      if (!cols[c] && !d1[r + c] && !d2[r - c + n]) {
        cols[c] <<- TRUE; d1[r + c] <<- TRUE; d2[r - c + n] <<- TRUE
        s <- s + place(r + 1)
        cols[c] <<- FALSE; d1[r + c] <<- FALSE; d2[r - c + n] <<- FALSE
      }
    }
    s
  }
  place(1)
}
cat(queens(8), "\n")
