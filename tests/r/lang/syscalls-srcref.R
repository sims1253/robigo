# sys.call()/sys.calls() report the srcref of the source statement a call
# runs under: eval.c:do_eval sets R_Srcref per statement of an expression
# vector, a closure entry switches to the closure's own srcref
# (eval.c:R_execClosure), and context.c:getCallWithSrcref attaches the
# frame's saved srcref to the reported call. testthat reads it for the
# ('file:line:col') prefix of every test failure.
show <- function(calls) {
  for (c in calls) {
    sr <- attr(c, "srcref")
    cat(deparse(c[[1]])[1], "->",
        if (is.null(sr)) "none" else paste(sr[1], sr[2], sr[4], sep = ":"),
        "\n")
  }
}
code <- paste(
  "inner <- function() sys.calls()",
  "outer <- function() inner()",
  "show(outer())",
  "show(sys.calls())",
  sep = "\n")
t <- tempfile(fileext = ".R")
writeLines(code, t)
source(t, keep.source = TRUE)
show(sys.call()) # top level: no statement srcref
eval(parse(text = "show(sys.calls())", keep.source = TRUE)[[1]], new.env())
# a `{` block's statements carry their own srcrefs (eval.c:do_begin reads
# the block's srcref list), and the block leaves none active afterwards
blk <- tempfile(fileext = ".R")
writeLines(c(
  "deep <- function() sys.calls()",
  "blocky <- function() {",
  "  x <- 1",
  "  deep()",
  "}"
), blk)
source(blk, keep.source = TRUE)
show(blocky())
show(sys.calls())
