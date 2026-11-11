# R: eval.c:applydefine binds `*tmp*` locked while the setter's arguments
# are evaluated: an argument that assigns into the target errors.
f <- function() {
  a <- list(b = 1:3)
  r <- try(a$b[{ a$b <- 10:12; 1 }] <- 0L, silent = TRUE)
  cat(r)
  print(a)
}
f()
