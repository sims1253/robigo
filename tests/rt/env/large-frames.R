# Frames with more bindings than robigo's slot locations used to encode
# (2^15): a function with 40,000 formals, and a function frame grown past
# 32,768 variables by assign.
fs <- rep(list(1), 40000)
names(fs) <- paste0("x", 1:40000)
fs[[40000]] <- 3
f <- eval(as.call(list(quote(`function`), as.pairlist(fs), quote(x40000 + x20000 + x1))))
for (k in 1:3) print(f())
print(f(x40000 = 10, x1 = 5))
g <- function() {
  for (i in 1:33000) assign(paste0("v", i), i)
  c(v1, v32768, v33000, get("v32769"))
}
print(g())
# the most formals a call matches (65,535): defaults and a named argument
fs <- rep(list(1), 65535)
names(fs) <- paste0("x", 1:65535)
h <- eval(as.call(list(quote(`function`), as.pairlist(fs), quote(x65535 + x1))))
print(h())
print(h(x65535 = 42))
