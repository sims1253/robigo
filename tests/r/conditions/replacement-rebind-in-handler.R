# R code a replacement runs sees the target bound (eval.c:applydefine),
# and a binding that R code changes is kept when the replacement fails.

# A symbol target has no object to modify in place: it stays bound.
x <- quote(foo)
as.character.zz <- function(z, ...) { print(x); "a" }
try(names(x) <- structure(1, class="zz"), silent=TRUE)
print(x)

# The same for a symbol inside a list.
a <- list(s = quote(foo))
as.character.zz <- function(z, ...) { print(a$s); "a" }
try(names(a$s) <- structure(1, class="zz"), silent=TRUE)
print(a)

# A handler rebinds the target, then the replacement fails: the handler's
# binding stays.
x <- structure("bad", class="foo")
tryCatch(
  withCallingHandlers(
    class(x) <- "integer",
    warning=function(w) {
      attr(x, "hello") <<- 42
      stop("boom")
    }
  ),
  error=function(e) NULL
)
print(x)

# The replacement succeeds: its result is assigned over the handler's.
x <- structure("bad", class="foo")
withCallingHandlers(
  class(x) <- "integer",
  warning=function(w) {
    attr(x, "hello") <<- 42
    invokeRestart("muffleWarning")
  }
)
print(x)

# A handler removes the target, then the replacement fails.
x <- structure("bad", class="foo")
tryCatch(
  withCallingHandlers(
    class(x) <- "integer",
    warning=function(w) {
      rm(x, envir=globalenv())
      stop("boom")
    }
  ),
  error=function(e) NULL
)
print(exists("x"))

# A handler rebinds the root of a nested target, then the replacement
# fails. (In a function: at top level `.Last.value` also holds the list.)
local({
  a <- list(b = structure("bad", class="foo"))
  tryCatch(
    withCallingHandlers(
      class(a$b) <- "integer",
      warning=function(w) {
        a$extra <<- 1
        stop("boom")
      }
    ),
    error=function(e) NULL
  )
  print(a)
})

# The same, and the replacement succeeds.
local({
  a <- list(b = structure("bad", class="foo"))
  withCallingHandlers(
    class(a$b) <- "integer",
    warning=function(w) {
      a$extra <<- 1
      invokeRestart("muffleWarning")
    }
  )
  print(a)
})

# An environment target stays bound while an active binding's function
# runs.
e <- new.env()
makeActiveBinding("a", function(v) { if (!missing(v)) print(class(e)); 1 }, e)
e$a <- 5
g <- function() {
  h <- new.env()
  makeActiveBinding("a", function(v) { if (!missing(v)) print(is.environment(h)); 1 }, h)
  h$a <- 5
}
g()
