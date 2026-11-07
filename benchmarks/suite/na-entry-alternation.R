# Alternate two NA-bearing entry slots. The empty cat keeps the caller in
# the VM while its numeric loop runs natively. The recurrence avoids a
# closed-form loop summary.
f <- function(a, b, x) {
  cat("")
  s <- 0
  for (i in seq_along(x)) {
    u <- a + 1L
    v <- b + 1L
    s <- sqrt(s + x[i]) + u + v + i * 3L
  }
  s
}
x <- seq_len(1500) / 1500
invisible(f(1L, 2L, x))
for (k in 1:512) {
  if (k %% 2L) r <- f(NA_integer_, 2L, x)
  else r <- f(1L, NA_integer_, x)
}
cat(is.na(r), is.nan(r), "\n")
