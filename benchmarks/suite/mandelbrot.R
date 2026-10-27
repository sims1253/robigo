# Mandelbrot set membership count, scalar complex-free arithmetic.
mandel <- function(n) {
  count <- 0L
  for (py in 0:(n - 1)) {
    ci <- 2 * py / n - 1
    for (px in 0:(n - 1)) {
      cr <- 2 * px / n - 1.5
      zr <- 0; zi <- 0; i <- 0L
      while (i < 50L && zr * zr + zi * zi <= 4) {
        t <- zr * zr - zi * zi + cr
        zi <- 2 * zr * zi + ci
        zr <- t
        i <- i + 1L
      }
      if (i == 50L) count <- count + 1L
    }
  }
  count
}
cat(mandel(120), "\n")
