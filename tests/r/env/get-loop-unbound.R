# get() in a compiled for loop: a hit followed by an unbound name must
# error, never return an earlier iteration's value (issue #224). The miss
# name is bound elsewhere, in an object's environment that compiled code
# reaches with `$`, like rlang's `lines$make_next_sticky()`.
ns <- new.env(parent = baseenv())
local(envir = ns, {
  new_lines <- function() {
    self <- environment()
    sticky <- FALSE
    make_next_sticky <- function() self$sticky <- TRUE
    push <- function(x) if (self$sticky) paste0("~", x) else x
    self
  }
  op_deparse <- function(x, lines) {
    lines$make_next_sticky()
    lines$push(x)
  }
  tag <- "a string"
})
lines <- ns$new_lines()
for (k in 1:200) out <- ns$op_deparse("x", lines)
out
miss <- function(e) conditionMessage(e)

# Top level, then inside a function, then many rounds.
for (n in c("op_deparse", "make_next_sticky")) print(tryCatch(class(get(n, envir = ns)), error = miss))
for (n in c("tag", "op_deparse", "new_lines", "make_next_sticky")) print(tryCatch(class(get(n, envir = ns)), error = miss))
f <- function(names) for (n in names) print(tryCatch(class(get(n, envir = ns)), error = miss))
f(c("op_deparse", "make_next_sticky"))
f(c("tag", "make_next_sticky", "op_deparse", "make_next_sticky"))
r <- character()
for (k in 1:300) for (n in c("op_deparse", "make_next_sticky")) r <- c(r, tryCatch(class(get(n, envir = ns)), error = function(e) "error"))
table(r)

# The same names through get0, exists and mget.
for (n in c("op_deparse", "make_next_sticky")) print(class(get0(n, envir = ns, ifnotfound = NULL)))
for (n in c("op_deparse", "make_next_sticky")) print(exists(n, envir = ns))
for (n in c("op_deparse", "make_next_sticky")) print(class(mget(n, envir = ns, ifnotfound = list(NULL))[[1]]))

# A binding removed between iterations is gone on the next lookup.
e <- new.env(parent = baseenv())
for (k in 1:3) {
  if (k == 2) rm("v", envir = e) else assign("v", k, envir = e)
  print(tryCatch(get("v", envir = e), error = miss))
}

# The issue's loop, uncaught: GNU R stops at the second name.
for (n in c("op_deparse", "make_next_sticky")) print(class(get(n, envir = ns)))
