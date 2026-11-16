# Frames with more bindings than robigo's slot locations used to encode
# (slot indices below 2^15), just past that limit: a function of 32,769
# formals (the last one's slot index is 2^15), and a function frame grown
# to 32,769 variables beyond its shape by assign. The variables' names are
# made once, as the formals' tags, and the frame grows through
# .Internal(assign()) with a constant: nothing is allocated per variable,
# which under gctorture would cost a collection each.
fs <- rep(list(1), 32769)
names(fs) <- paste0("x", 1:32769)
fs[[32769]] <- 3
f <- eval(as.call(list(quote(`function`), as.pairlist(fs), quote(c(x32769 + x16384 + x1, get("x32769"))))))
for (k in 1:3) print(f())
print(f(x32769 = 10, x1 = 5))
g <- function(nms) {
  e <- environment()
  for (nm in nms) .Internal(assign(nm, 1, e, FALSE))
  .Internal(assign("x32768", 32768, e, FALSE))
  .Internal(assign("x32769", 32769, e, FALSE))
  c(x1, x32768, x32769, get("x32769"))
}
print(g(names(fs)))
# the most formals a call matches (65,535): defaults and a named argument
fs <- rep(list(1), 65535)
names(fs) <- paste0("x", 1:65535)
h <- eval(as.call(list(quote(`function`), as.pairlist(fs), quote(x65535 + x1))))
print(h())
print(h(x65535 = 42))
