# `~` (names.c do_tilde): the call itself, of class "formula", with the
# environment it was evaluated in.
fm <- y ~ x + z
fm
class(fm)
identical(environment(fm), globalenv())
names(attributes(fm))
length(fm)
fm[[1]]
fm[[3]]
typeof(fm)
one <- ~ x
length(one)
one[[2]]
# evaluated in a function: its frame
ff <- function() {
  e <- environment()
  identical(environment(a ~ b), e)
}
ff()
identical(environment(local(u ~ v)), globalenv())
# any number of arguments; nothing is evaluated
r <- `~`(1, 2, 3)
class(r)
length(r)
r0 <- `~`()
class(r0)
length(r0)
nested <- ~ (a ~ b)
class(nested[[2]])
class(nested[[2]][[2]])
undefined_ok <- this_is ~ not_defined
class(undefined_ok)
# a call that is already an object is only duplicated
q <- structure(quote(a ~ b), class = "foo")
r <- eval(q)
oldClass(r)
is.null(attr(r, ".Environment"))
# the result is a copy: the code is not changed
g <- function() y ~ x
a <- g()
attr(a, "extra") <- 1
names(attributes(g()))
# through do.call and match.fun
d <- do.call("~", list(quote(a), quote(b)))
class(d)
identical(environment(d), globalenv())
tf <- match.fun("~")
class(tf(p, q))
typeof(`~`)
lapply(1:2, function(i) class(~ i))
