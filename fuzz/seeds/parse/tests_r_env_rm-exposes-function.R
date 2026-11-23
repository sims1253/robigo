# rm() of the first of a repeated tag makes the next binding visible; when
# that one is a function, calls made after the removal (in a loop the
# bytecode compiler caches lookups in) must find it.
pad <- structure(as.list(1:20), names = paste0("v", 1:20))

# a small frame and one large enough to be indexed
for (frame in list(list(), pad)) {
  # a plain value hides a function binding of `+`
  eval(quote(for (i in 1:3) {
    print(1 + 2)
    if (i == 1) rm(`+`)
  }), c(frame, list(`+` = 0, `+` = function(a, b) 99)))

  # ... and of a closure called by name
  f <- function() "global"
  eval(quote(for (i in 1:3) {
    print(f())
    if (i == 1) rm(f)
  }), c(frame, list(f = 0, f = function() "local")))

  # removing the function exposes a plain value: back to the global one
  eval(quote(for (i in 1:3) {
    print(f())
    if (i == 1) rm(f)
  }), c(frame, list(f = function() "local", f = 0)))

  # three bindings, removed one at a time
  eval(quote(for (i in 1:4) {
    print(f())
    if (i < 4) rm(f)
  }), c(frame, list(f = function() "one", f = 0, f = function() "three")))
}
