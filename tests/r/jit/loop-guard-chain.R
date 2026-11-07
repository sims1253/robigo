# Re-enter a hot loop after replacing operators in each lookup frame.
make <- function(depth) {
  e <- new.env(parent = globalenv())
  target <- e
  for (i in seq_len(depth)) e <- new.env(parent = e)
  e$target <- target
  eval(quote(function() {
    s <- 0
    for (k in 1:8) {
      for (j in 1:600) s <- s + j
      if (k == 3) assign("+", function(a, b) a - b, envir = target)
      if (k == 5) rm("+", envir = target)
    }
    s
  }), e)
}
for (d in 0:4) {
  f <- make(d)
  print(f())
}
# A locked frame can still have its existing bindings replaced.
e <- new.env(parent = globalenv())
e$`+` <- base::`+`
lockEnvironment(e)
f <- eval(quote(function() {
  s <- 0
  for (k in 1:8) {
    for (j in 1:600) s <- s + j
    if (k == 3) assign("+", function(a, b) a - b, envir = environment(f))
    if (k == 5) assign("+", base::`+`, envir = environment(f))
  }
  s
}), e)
print(f())
