# n-body simulation (Computer Language Benchmarks Game), scalar arithmetic
# on short numeric vectors.
nbody <- function(n) {
  pi <- 3.141592653589793; solar <- 4 * pi * pi; days <- 365.24
  x <- c(0, 4.84143144246472090e+00, 8.34336671824457987e+00, 1.28943695621391310e+01, 1.53796971148509165e+01)
  y <- c(0, -1.16032004402742839e+00, 4.12479856412430479e+00, -1.51111514016986312e+01, -2.59193146099879641e+01)
  z <- c(0, -1.03622044471123109e-01, -4.03523417114321381e-01, -2.23307578892655734e-01, 1.79258772950371181e-01)
  vx <- c(0, 1.66007664274403694e-03, -2.76742510726862411e-03, 2.96460137564761618e-03, 2.68067772490389322e-03) * days
  vy <- c(0, 7.69901118419740425e-03, 4.99852801234917238e-03, 2.37847173959480950e-03, 1.62824170038242295e-03) * days
  vz <- c(0, -6.90460016972063023e-05, 2.30417297573763929e-05, -2.96589568540237556e-05, -9.51592254519715870e-05) * days
  m <- c(1, 9.54791938424326609e-04, 2.85885980666130812e-04, 4.36624404335156298e-05, 5.15138902046611451e-05) * solar
  vx[1] <- -sum(vx * m) / solar; vy[1] <- -sum(vy * m) / solar; vz[1] <- -sum(vz * m) / solar
  energy <- function() {
    e <- 0
    for (i in 1:5) {
      e <- e + 0.5 * m[i] * (vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i])
      if (i < 5) for (j in (i + 1):5) {
        dx <- x[i] - x[j]; dy <- y[i] - y[j]; dz <- z[i] - z[j]
        e <- e - m[i] * m[j] / sqrt(dx * dx + dy * dy + dz * dz)
      }
    }
    e
  }
  e0 <- energy()
  dt <- 0.01
  for (step in 1:n) {
    for (i in 1:4) for (j in (i + 1):5) {
      dx <- x[i] - x[j]; dy <- y[i] - y[j]; dz <- z[i] - z[j]
      d2 <- dx * dx + dy * dy + dz * dz
      mag <- dt / (d2 * sqrt(d2))
      vx[i] <- vx[i] - dx * m[j] * mag; vy[i] <- vy[i] - dy * m[j] * mag; vz[i] <- vz[i] - dz * m[j] * mag
      vx[j] <- vx[j] + dx * m[i] * mag; vy[j] <- vy[j] + dy * m[i] * mag; vz[j] <- vz[j] + dz * m[i] * mag
    }
    for (i in 1:5) { x[i] <- x[i] + dt * vx[i]; y[i] <- y[i] + dt * vy[i]; z[i] <- z[i] + dt * vz[i] }
  }
  c(e0, energy())
}
cat(sprintf("%.9f", nbody(20000)), "\n")
