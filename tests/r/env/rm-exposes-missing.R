# rm() of the first of a repeated tag can expose a missing argument; a
# function lookup stops there with an error, so calls made after the
# removal (in a loop the bytecode compiler caches lookups in) must see it.
pad <- structure(as.list(1:20), names = paste0("v", 1:20))

# a small frame and one large enough to be indexed
for (frame in list(list(), pad)) {
  r <- try(eval(quote(for (i in 1:3) {
    print(1 + 2)
    if (i == 1) rm(`+`)
  }), c(frame, list(`+` = 0), alist(`+` = ))), silent = TRUE)
  cat(class(r), conditionMessage(attr(r, "condition")), "\n")
}
