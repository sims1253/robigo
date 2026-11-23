# `x[[i]] <- v` on a variable bound to a builtin, with a two-element index
# (found by scripts/fuzz-eval.py --tiers): GNU R reports the assignment
# call ("should only be called in an assignment context."); robigo reports
# `[[<-`'s vector index error instead.
x <- list
x[[length(x) + c(1L, 1L)]] <- NA_complex_
