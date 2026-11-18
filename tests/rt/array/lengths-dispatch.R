# array.c do_lengths on objects: each element's length through its
# `length` method (dispatch_xlength) and, for a classed list, each element
# through `[[` (dispatch_subset2); `lengths` itself is an internal generic.
show <- function(expr) {
  r <- tryCatch(expr, error = function(e) e)
  if (inherits(r, "error")) {
    cat("error:", conditionMessage(r), "| call:", deparse(conditionCall(r)), "\n")
  } else {
    cat(typeof(r), ":", r, "\n")
    a <- attributes(r)
    for (n in names(a)) cat(" ", n, ":", if (is.list(a[[n]])) unlist(a[[n]]) else a[[n]], "\n")
  }
  invisible()
}
length.three <- function(x) 3L
length.half <- function(x) 2.5
length.neg <- function(x) -1L
length.big <- function(x) 3e9
length.chr <- function(x) "7"
length.na <- function(x) NA
el <- function(cls) structure(list(), class = cls)
# a list of classed elements: each element's length method
show(lengths(list(a = el("three"), b = 1:4, c = el("half"))))
show(lengths(list(el("neg"), 1)))
show(lengths(list(el("chr"))))
show(lengths(list(el("na"))))
show(lengths(list(el("big"))))
show(lengths(list(a = el("three")), use.names = FALSE))
# a classed list: its length method for the count, `[[` for each element
k <- structure(list(1:2, letters), class = "kl")
`[[.kl` <- function(x, i) { cat("[[ ", i, "\n"); seq_len(i * 10) }
show(lengths(k))
length.kl <- function(x) 3L
show(lengths(k))
names(k) <- c("p", "q")
show(lengths(k))
show(lengths(k, use.names = FALSE))
# the call `[[` and `length` methods see
`[[.kc` <- function(x, i) { cat(deparse(sys.call()), "\n"); 1 }
length.kc <- function(x) { cat(deparse(sys.call()), "\n"); 1L }
show(lengths(structure(list(0), class = "kc")))
# a classed atomic vector: every element has length 1, the count by method
show(lengths(structure(1:3, class = "three")))
show(lengths(structure(1:3, class = "neg")))
# a lengths method
lengths.lm1 <- function(x, use.names = TRUE) "method"
show(lengths(structure(list(1), class = "lm1")))
show(lengths(structure(1, class = "half"), use.names = NA))
