# E9 (docs/g3.md §15.3, §3 item 5): visible effects (cat, a binding store,
# a warning) between allocations of several sizes. Run with
# ROBIGO_FAIL_ALLOC_AT=n (scripts/rt-fail-alloc.py), every allocation in
# turn is refused once and retried: the output must not change, so no
# effect is lost or repeated.
x <- 0
acc <- character()
for (i in 1:12) {
    cat("step", i, "\n")
    x <- x + i
    v <- rep(0, i * 1000)
    acc[i] <- paste("s", i)
    if (i %% 4 == 0) warning("a warning at ", i)
    l <- list(v, x, acc)
    names(l) <- c("v", "x", "acc")
}
cat("x =", x, "length", length(l$acc), sum(l$v), "\n")
print(acc)
