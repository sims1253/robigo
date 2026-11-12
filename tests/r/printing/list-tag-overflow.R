# Printing deeply nested lists: once the element tag would pass 256 bytes R
# appends "$..." once and then nothing (print.c:printValueRec, TAGBUFLEN).
# An index tag is checked with IndexWidth(i) of the 0-based index, which is
# 0 for the first element, and an NA name is checked as "NA".
tags <- function(x) {
    tc <- textConnection("out", "w", local = TRUE)
    sink(tc); print(x); sink(); close(tc)
    unique(out[grepl("^[$[]", out)])
}
nest <- function(depth, key) {
    ch <- if (depth == 1L) list() else list(nest(depth - 1L, key))
    r <- list()
    r[[key]] <- ch
    r
}
# the reg-tests-1c case: "$..." brings the tag to exactly 256 bytes, and
# the [[1]] after it still fits (IndexWidth(0) is 0)
t <- tags(nest(20, "children"))
nchar(t)
t[length(t) - 1:0]
# a name after a 256-byte tag
t <- tags(nest(40, "ab"))
nchar(t)
t[length(t) - 1:0]
t <- tags(nest(40, "abc"))
nchar(t)
t[length(t) - 1:0]
# unnamed nesting
deep <- function(d, leaf) if (d == 0L) leaf else list(deep(d - 1L, leaf))
t <- tags(deep(60, 1))
nchar(t)
# an NA name past the limit
leaf <- list(1, 2); names(leaf) <- c(NA, "b")
t <- tags(deep(52, leaf))
t[length(t) - 2:0]
t <- tags(deep(50, leaf))
t[length(t) - 2:0]
# pairlists (printList) measure the 1-based index, so the [[1]] after the
# 256-byte "$..." does not fit
pnest <- function(depth) {
    ch <- if (depth == 1L) NULL else pairlist(pnest(depth - 1L))
    pairlist(children = ch)
}
t <- tags(pnest(20))
nchar(t)
t[length(t) - 1:0]
